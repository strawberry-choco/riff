// riff-infra — Application Store adapter tests (real SQLite).
//
// Moved verbatim from the workspace-root suite's `infra_tests` module by
// backend-crate-split issue 07: the Application Store adapter and its tests
// belong to `riff-infra`. Import paths were rewired to `riff-persistence` /
// `riff-infra`; every assertion is unchanged.

use super::*;

// --- Application Store: connection setup + checksummed migrations --------

#[test]
fn test_store_fresh_start_creates_file_and_applies_initial_migration_once() {
    let (_dir, _, store, _changes_rx) = open_store_with_path();

    let applied: Vec<(i64, String)> = store
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect()
        })
        .expect("reading schema_migrations must work");
    // The shipped initial set: v1 (foundation) + v2 (typed settings tables)
    // + v3 (playlists) + v4 (library collection) + v5 (playback prefs)
    // + v6 (track favorites) + v7 (browser layout) + v8 (library scan prefs)
    // + v9 (smart lists collapsed) + v10 (drop the missing-artwork strategy)
    // + v11 (lowercased entity search keys) + v12 (retire the browser layout)
    // + v13 (the quit-on-close preference) + v14 (ReplayGain album gain)
    // + v15 (the metadata version the freshness filter compares against
    //   `METADATA_VERSION`) + v16 (ReplayGain album peak) + v17 (the
    //   ReplayGain Mode preference) + v18 (the pass's Settings gating)
    //   + v19 (the Reduce Motion accessibility preference) + v20 (the Custom
    //   Smart Playlist rule tables).
    assert_eq!(
        applied.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20
        ]
    );
}

#[test]
fn test_store_reopen_reuses_migrated_store_without_reapplying() {
    let (_dir, db_path, store, _changes_rx) = open_store_with_path();
    store
        .with_connection(|conn| conn.execute("UPDATE schema_migrations SET applied_at = 12345", []))
        .expect("marking applied_at must work");
    drop(store);

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening a migrated store must succeed");
    let applied_at: i64 = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT applied_at FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get(0),
            )
        })
        .expect("reading applied_at must work");
    assert_eq!(
        applied_at, 12345,
        "reopening must reuse the recorded migration row, not reapply"
    );
}

#[test]
fn test_store_double_apply_is_idempotent() {
    let (_dir, mut store, _changes_rx) = open_store();
    store
        .with_connection(|conn| conn.execute("UPDATE schema_migrations SET applied_at = 777", []))
        .expect("marking applied_at must work");

    // Applying the full migration set a second time in sequence must be
    // a no-op over the bookkeeping rows.
    store.apply_migrations().expect("second apply must succeed");

    let rows: Vec<(i64, String)> = store
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")?;
            let mapped = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            mapped.collect()
        })
        .expect("reading schema_migrations must work");
    assert_eq!(rows.len(), 20, "no duplicate migration rows allowed");
    assert_eq!(
        rows.iter().map(|(v, _)| *v).collect::<Vec<_>>(),
        vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20
        ]
    );

    let applied_at: i64 = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT applied_at FROM schema_migrations WHERE version = 1",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(
        applied_at, 777,
        "idempotent re-apply must not touch existing rows"
    );
}

/// Migration 010 removes the missing-artwork strategy column (the generated
/// colour placeholder feature). Simulate a store that predates the migration
/// — un-apply 010 and re-add the old column — then reopen through the full
/// migration path: the column must drop and the surviving scalars must carry
/// over untouched.
#[test]
fn test_store_migration_010_drops_the_missing_artwork_strategy_column() {
    // A fully migrated fresh store is the base: every scalar at defaults.
    let (_dir, db_path, store, _changes_rx) = open_store_with_path();

    // Roll back to the 009-era state: forget migration 010 and restore the
    // columns exactly as migrations 007/008 created them, plus some
    // non-default scalars that 010's table rebuild must preserve. The later
    // app_settings-column migrations (013 `close_quits_app`, 015 the
    // metadata version, 017 the ReplayGain Mode, 018 the pass's Settings
    // gating, 019 Reduce Motion) are also un-applied, because
    // 010 rebuilds the settings row from a column snapshot predating them;
    // re-applying them on the way back keeps the newest scalars present for
    // reads.
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "DELETE FROM schema_migrations WHERE version IN (10, 13, 15, 17, 18, 19);
                 ALTER TABLE app_settings
                   ADD COLUMN missing_artwork_strategy TEXT NOT NULL DEFAULT 'generated_colour'
                     CHECK (missing_artwork_strategy IN ('generated_colour'));
                 ALTER TABLE app_settings
                   ADD COLUMN browser_layout INTEGER NOT NULL DEFAULT 0
                     CHECK (browser_layout IN (0, 1));
                 UPDATE app_settings
                   SET volume = 0.42, shuffle = 1, smart_lists_collapsed = 1;",
            )
        })
        .expect("rolling back to the 009-era schema must work");
    drop(store);

    // Reopening applies only the pending migration 010: the table is rebuilt
    // without the column and the remaining scalars ride along.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let upgraded = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let columns: Vec<String> = upgraded
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('app_settings')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("reading the settings table's columns must work");
    assert!(
        !columns
            .iter()
            .any(|column| column == "missing_artwork_strategy"),
        "migration 010 must drop the missing-artwork column"
    );

    let scalars = upgraded
        .load_settings()
        .expect("loading settings from the upgraded store must work")
        .scalars;
    assert_eq!(
        scalars.volume,
        Some(0.42),
        "the rebuild must preserve volume"
    );
    assert!(scalars.shuffle, "the rebuild must preserve shuffle");
    assert!(
        scalars.smart_lists_collapsed,
        "the rebuild must preserve smart-lists collapsed"
    );
}

/// Migration 011 adds write-time-lowercased key columns to the entity
/// tables (`artists.name_lower`, `albums.album_artist_lower` /
/// `albums.title_lower`) so entity name matching can be case-insensitive —
/// `SQLite`'s `instr` is case-sensitive, and the tracks' derived `search_text`
/// is the same precedent. Simulate a store that predates the migration —
/// un-apply 011 and drop the columns — then reopen through the full
/// migration path: the columns must come back and existing rows must be
/// backfilled lowercased.
#[test]
fn test_store_migration_011_backfills_lowercased_entity_search_keys() {
    // A fully migrated fresh store is the base.
    let (_dir, db_path, store, _changes_rx) = open_store_with_path();

    // Roll back to the 010-era schema: forget migration 011 and drop its
    // columns, then seed entity rows exactly as a pre-011 scan would have.
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "DELETE FROM schema_migrations WHERE version = 11;
                 ALTER TABLE artists DROP COLUMN name_lower;
                 ALTER TABLE albums DROP COLUMN album_artist_lower;
                 ALTER TABLE albums DROP COLUMN title_lower;
                 INSERT INTO artists(name) VALUES ('The Zeta');
                 INSERT INTO albums(album_artist, title, year, genre)
                   VALUES ('The Zeta', 'New Wave', 2019, 'Rock');",
            )
        })
        .expect("rolling back to the 010-era schema must work");
    drop(store);

    // Reopening applies only the pending migration 011: the lowercased key
    // columns come back and the pre-existing rows are backfilled.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let upgraded = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let (name_lower, album_artist_lower, title_lower): (String, String, String) = upgraded
        .with_connection(|conn| {
            conn.query_row(
                "SELECT a.name_lower, al.album_artist_lower, al.title_lower
                 FROM artists a JOIN albums al ON al.album_artist = a.name
                 WHERE a.name = 'The Zeta'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
        })
        .expect("the backfilled columns must be readable");
    assert_eq!(name_lower, "the zeta", "artist names backfill lowercased");
    assert_eq!(
        album_artist_lower, "the zeta",
        "album artist keys backfill lowercased"
    );
    assert_eq!(title_lower, "new wave", "album titles backfill lowercased");
}

/// The scan/tag-edit write path derives the lowercased key columns in Rust,
/// so non-Latin names stay case-insensitive exactly like `search_text` (a
/// SQL-only backfill could only fold ASCII).
#[test]
fn test_store_scan_writes_rust_lowercased_entity_search_keys() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(std::slice::from_ref(&browsing_track(
            "f:\\cyg\\1.mp3",
            "Лебединое",
            "Чайковский",
            "Балеты",
            Some(1),
            Some(1890),
        )))
        .expect("batch applies");

    let (name_lower, title_lower): (String, String) = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT a.name_lower, al.title_lower
                 FROM artists a JOIN albums al ON al.album_artist = a.name",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("the derived columns must be readable");
    assert_eq!(name_lower, "чайковский", "Rust-lowercased artist key");
    assert_eq!(title_lower, "балеты", "Rust-lowercased album title");
}

/// A checksum mismatch is an integrity signal, not corruption: the store
/// opened, answered, and disagreed with this build about what its own schema
/// history is. It is answered by refusing to open, so the file the user
/// already has is exactly the file they are left with.
///
/// Driven through the real open path on a real file — opened, tampered,
/// closed, re-opened — because the migration-apply path on an already-open
/// store never reaches the open decision at all and so cannot observe what a
/// user gets.
#[test]
fn test_store_checksum_tamper_is_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // A populated store, closed again: the real user's starting point.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("fresh store must open and migrate");
        store
            .apply_scan_batch(&[
                library_track("m:\\lib\\a.mp3", "A", None, "One", None),
                library_track("m:\\lib\\b.mp3", "B", None, "One", None),
            ])
            .expect("batch applies");
    }

    // Simulate a tampered bookkeeping row: the recorded checksum no longer
    // matches what this build derives from that migration's own SQL.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening the populated store must work");
        store
            .with_connection(|conn| {
                conn.execute("UPDATE schema_migrations SET checksum = 'deadbeef'", [])
            })
            .expect("tampering with the checksum must work");
    }
    let before = std::fs::read(&db_path).expect("the tampered store file must be readable");

    // Reopen exactly as a user relaunching the app does.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let Err(err) = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx) else {
        panic!("a checksum mismatch must refuse to open, but open succeeded");
    };
    let message = err.to_string();
    assert!(
        message.contains("tampered"),
        "error must clearly name the checksum tamper: {message}"
    );
    assert!(
        message.contains("refused to open"),
        "error must say the store was not opened, not that it was rebuilt: {message}"
    );

    // No rename-aside: an integrity signal never empties the store the way
    // corruption recovery does.
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let asides: Vec<&String> = entries
        .iter()
        .filter(|name| name.starts_with("riff.sqlite3."))
        .collect();
    assert!(
        asides.is_empty(),
        "a checksum mismatch must leave the store file alone, not rename it aside: {entries:?}"
    );

    // And the file is byte-for-byte what it was: no new empty store built
    // over the top of it, and the tampered bookkeeping row still there.
    assert_eq!(
        std::fs::read(&db_path).expect("the store file must still be readable"),
        before,
        "a refused open must leave the store file exactly as it was"
    );
    let tampered: i64 =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("the untouched store must still be a readable database")
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE checksum = 'deadbeef'",
                [],
                |row| row.get(0),
            )
            .expect("the tampered bookkeeping row must still be there");
    assert_eq!(
        tampered, 20,
        "the store's own migration rows are untouched: nothing re-applied, nothing replaced"
    );
}

/// Every recorded checksum is a fact of its own migration's SQL. The store
/// records the digest this build derives from a migration's SQL text, so the
/// two can only ever agree, and an edited migration changes what the store
/// expects.
///
/// This is the assertion whose absence let a hand-typed digest of the literal
/// string `"test"` ship as migration 1's checksum: nothing ever compared a
/// recorded value against the SQL it was supposed to describe.
#[test]
fn test_store_records_each_migrations_own_derived_checksum() {
    let (_dir, _, store, _changes_rx) = open_store_with_path();

    let recorded: Vec<(i64, String)> = store
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT version, checksum FROM schema_migrations ORDER BY version")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect()
        })
        .expect("reading schema_migrations must work");

    let migrations = riff_infra::store::sqlite::embedded_migrations();
    assert_eq!(
        recorded.len(),
        migrations.len(),
        "every embedded migration records a checksum, and no others do"
    );

    for ((version, checksum), migration) in recorded.iter().zip(migrations) {
        assert_eq!(
            *version, migration.version,
            "recorded versions are the embedded ones, in order"
        );
        assert_eq!(
            checksum,
            &test_derived_digest(migration.sql),
            "migration {version} ({}) must record the digest of its OWN sql, not a typed literal",
            migration.name
        );
        assert_eq!(
            &migration.checksum(),
            checksum,
            "the adapter's own derivation of migration {version} ({}) agrees",
            migration.name
        );
        assert!(
            checksum.len() == 64
                && checksum
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "migration {version} ({}) must record 64 lowercase hex characters, got {checksum:?}",
            migration.name
        );
    }

    let distinct: std::collections::HashSet<&String> = recorded.iter().map(|(_, c)| c).collect();
    assert_eq!(
        distinct.len(),
        recorded.len(),
        "no two migrations may share a digest — a repeated one is a copied one"
    );
}

/// The test's own implementation of the normalisation the adapter documents
/// (`CRLF`/`CR` to `LF`, trailing whitespace stripped from every line, leading
/// and trailing blank lines dropped, exactly one trailing newline), hashed with
/// `SHA-256` and encoded as lowercase hex. Written out here rather than called
/// so the assertion pins the rule instead of running the production function
/// against itself.
fn test_derived_digest(sql: &str) -> String {
    use sha2::{Digest, Sha256};

    const HEX: &[u8; 16] = b"0123456789abcdef";

    let unified = sql.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = unified.split('\n').map(str::trim_end).collect();
    let first = lines
        .iter()
        .position(|line| !line.is_empty())
        .unwrap_or(lines.len());
    let last = lines
        .iter()
        .rposition(|line| !line.is_empty())
        .map_or(first, |index| index + 1);
    let mut normalized = lines[first..last].join("\n");
    normalized.push('\n');

    let digest = Sha256::digest(normalized.as_bytes());
    digest
        .iter()
        .flat_map(|byte| [HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 0x0f)]])
        .map(char::from)
        .collect()
}

#[test]
fn test_metadata_version_lags_a_freshly_migrated_store_and_stamps_durably() {
    use riff_persistence::track::METADATA_VERSION;

    let (_dir, db_path, mut store, _changes_rx) = open_store_with_path();

    // A store that has never completed a scan under this build sits behind the
    // binary — that gap is what makes the scan's freshness filter re-read
    // every known path exactly once instead of forever.
    let lagging = store.metadata_version().expect("version reads");
    assert_eq!(lagging, 0);
    assert_ne!(
        lagging, METADATA_VERSION,
        "a just-migrated store must sit behind the binary, or nothing ever backfills"
    );

    store
        .stamp_metadata_version(METADATA_VERSION)
        .expect("stamping forward is a plain write");
    assert_eq!(
        store.metadata_version().expect("version reads"),
        METADATA_VERSION
    );

    // Durable: the stamp is the whole reason the re-read is paid once.
    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    assert_eq!(
        reopened.metadata_version().expect("version reads"),
        METADATA_VERSION,
        "the stamped version survives a reopen"
    );
}

#[test]
fn test_store_fresh_start_creates_its_own_parent_directories() {
    // SQLite creates a missing database file but never the directory above it,
    // and a first launch has no data directory yet, so opening a Store below
    // missing parent directories must succeed rather than report the raw
    // "unable to open database file" that a plain file open would give.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("nope/should/be/created/riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let _store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("a fresh store below missing parent directories must be created and opened");

    assert!(
        db_path.parent().expect("store path has a parent").is_dir(),
        "every parent directory must have been created"
    );
    assert!(
        db_path.is_file(),
        "the store file must have been created in them"
    );
}

#[test]
fn test_store_connection_setup_pragmas() {
    let (_dir, store, _changes_rx) = open_store();
    let journal_mode: String = store
        .with_connection(|conn| conn.query_row("PRAGMA journal_mode", [], |row| row.get(0)))
        .expect("reading journal_mode must work");
    assert_eq!(journal_mode.to_lowercase(), "wal");

    let synchronous: i64 = store
        .with_connection(|conn| conn.query_row("PRAGMA synchronous", [], |row| row.get(0)))
        .expect("reading synchronous must work");
    assert_eq!(synchronous, 1, "synchronous=NORMAL");

    let foreign_keys: i64 = store
        .with_connection(|conn| conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0)))
        .expect("reading foreign_keys must work");
    assert_eq!(foreign_keys, 1, "foreign_keys must be ON");
}

#[test]
fn test_store_busy_timeout_is_about_five_seconds() {
    let (_dir, store, _changes_rx) = open_store();
    let timeout_ms: i64 = store
        .with_connection(|conn| conn.query_row("PRAGMA busy_timeout", [], |row| row.get(0)))
        .expect("reading busy_timeout must work");
    assert!(
        (4_000..=6_000).contains(&timeout_ms),
        "busy timeout should be roughly five seconds, got {timeout_ms}ms"
    );
}

// --- Application Store: corruption detection + automatic recovery ----------

#[test]
fn test_store_corrupt_db_reopens_as_fresh_store_with_siblings_renamed_aside() {
    let (dir, db_path, store, _changes_rx) = open_store_with_path();
    let wal_path = dir.path().join("riff.sqlite3-wal");
    let shm_path = dir.path().join("riff.sqlite3-shm");

    // Build a healthy store with data, then close it so the WAL is
    store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO store_metadata(key, value) VALUES ('marker', 'pre-corruption')",
                [],
            )
        })
        .unwrap();
    drop(store);

    // Simulate crash leftovers: a stale WAL/SHM pair beside the database.
    std::fs::write(&wal_path, b"stale wal bytes").unwrap();
    std::fs::write(&shm_path, b"stale shm bytes").unwrap();

    // Corrupt the database bytes in place.
    let mut bytes = std::fs::read(&db_path).unwrap();
    assert!(
        bytes.len() > 200,
        "fixture must be large enough to corrupt meaningfully"
    );
    for byte in &mut bytes[100..200] {
        *byte = 0x00;
    }
    std::fs::write(&db_path, &bytes).unwrap();

    // Reopen: corruption must be detected at startup (quick_check) and
    // recovered automatically.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("corrupt store must be detected and replaced by a fresh one automatically");

    // Fresh start: pre-corruption data is intentionally not recovered.
    let marker: Option<String> = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT value FROM store_metadata WHERE key = 'marker'",
                [],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
        })
        .expect("fresh store must be queryable");
    assert!(marker.is_none(), "fresh start must not carry old data");

    // The corrupted database plus its -wal/-shm siblings were preserved
    // beside the originals with Unix-nanosecond suffixed names.
    let entries: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();

    let db_asides: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("riff.sqlite3."))
        .collect();
    assert!(
        !db_asides.is_empty(),
        "corrupt db must be renamed aside: {entries:?}"
    );
    for aside in &db_asides {
        let suffix = aside.trim_start_matches("riff.sqlite3.");
        assert!(
            !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()),
            "aside suffix must be Unix nanoseconds: {aside}"
        );
    }

    let wal_asides: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("riff.sqlite3-wal."))
        .collect();
    assert_eq!(
        wal_asides.len(),
        1,
        "exactly one renamed-aside wal expected: {entries:?}"
    );
    let wal_suffix = wal_asides[0].trim_start_matches("riff.sqlite3-wal.");
    assert!(
        !wal_suffix.is_empty() && wal_suffix.chars().all(|c| c.is_ascii_digit()),
        "wal aside suffix must be Unix nanoseconds: {}",
        wal_asides[0]
    );

    let shm_asides: Vec<&String> = entries
        .iter()
        .filter(|n| n.starts_with("riff.sqlite3-shm."))
        .collect();
    assert_eq!(
        shm_asides.len(),
        1,
        "exactly one renamed-aside shm expected: {entries:?}"
    );
}

#[test]
fn test_store_recovery_failure_is_a_fatal_startup_error() {
    // A store path occupied by a directory cannot be opened, cannot be renamed
    // aside as a corrupted file, and cannot be recreated in its place: recovery
    // itself must fail with a clear fatal error instead of crashing or silently
    // continuing. (A missing parent directory is NOT such a path — the Store
    // creates those, so it can no longer stand in for an unrecoverable store.)
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    std::fs::create_dir(&db_path).expect("a directory must occupy the store path");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let Err(err) = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx) else {
        panic!("recovery failure must be a fatal startup error, but open succeeded");
    };
    let message = err.to_string();
    assert!(
        message.contains("could not be recovered"),
        "error must clearly name the fatal recovery failure: {message}"
    );
}

// --- Application Store: Settings in typed tables -----------------------------

use riff_persistence::store::SettingsStore;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_store_scalar_settings_roundtrip_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Fresh store: defaults (volume unset, toggles off).
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let settings = store
            .load_settings()
            .expect("loading settings from a fresh store must work");
        assert_eq!(settings.scalars.volume, None);
        assert!(!settings.scalars.advanced_mode);
        assert!(!settings.scalars.high_contrast);
        assert!(!settings.scalars.replaygain_enabled);
        // The pass's gating defaults off, so a scan's behavior is unchanged
        // until the listener opts in. The ReplayGain Mode is deliberately
        // NOT a setting: the queue provenance decides it during playback.
        assert!(!settings.scalars.replaygain_pass_track);
        assert!(!settings.scalars.replaygain_pass_album);
        assert!(!settings.scalars.smart_lists_collapsed);
        assert!(!settings.scalars.close_quits_app);
    }

    // Change every scalar and drop the connection (the "restart").
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        store
            .save_scalars(&riff_persistence::store::ScalarSettings {
                volume: Some(0.42),
                advanced_mode: true,
                high_contrast: true,
                replaygain_enabled: true,
                replaygain_pass_track: true,
                replaygain_pass_album: true,
                shuffle: true,
                repeat_mode: 2,
                smart_lists_collapsed: true,
                close_quits_app: true,
                ..riff_persistence::store::ScalarSettings::default()
            })
            .expect("saving scalars must work");
    }

    // Reopen: every value survives in its typed column.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let settings = reopened
        .load_settings()
        .expect("loading settings must work");
    assert_eq!(settings.scalars.volume, Some(0.42));
    assert!(settings.scalars.advanced_mode);
    assert!(settings.scalars.high_contrast);
    assert!(settings.scalars.replaygain_enabled);
    assert!(settings.scalars.shuffle);
    assert_eq!(settings.scalars.repeat_mode, 2);
    assert!(settings.scalars.smart_lists_collapsed);
    assert!(settings.scalars.close_quits_app);
}

/// Migration 019 adds the Reduce Motion accessibility preference. A store
/// that predates it (forget the migration, drop the column) reopens with the
/// preference off — the column arrives with `DEFAULT 0`, exactly like every
/// other add-a-boolean migration (009/013/017), so an existing store's
/// behavior is unchanged until the listener opts in — and the value
/// round-trips both directions once it is set.
#[test]
fn test_store_migration_019_adds_the_reduce_motion_column() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // A fully migrated fresh store is the base; roll it back to the pre-019
    // state, exactly as an install that predated the migration would be.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        store
            .with_connection(|conn| {
                conn.execute_batch(
                    "DELETE FROM schema_migrations WHERE version = 19;
                     ALTER TABLE app_settings DROP COLUMN reduce_motion;",
                )
            })
            .expect("rolling back to the pre-019 schema must work");
    }

    // Reopening applies the pending migration 019: the column arrives with
    // DEFAULT 0, so the preference reads back off.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let upgraded = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening must succeed");
        let scalars = upgraded
            .load_settings()
            .expect("loading settings from the upgraded store must work")
            .scalars;
        assert!(
            !scalars.reduce_motion,
            "migration 019 defaults reduce_motion off, so an existing store keeps its motion"
        );
    }

    // Opt in, then "restart": the value survives in its typed column.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening for the save must succeed");
        store
            .save_scalars(&riff_persistence::store::ScalarSettings {
                reduce_motion: true,
                ..riff_persistence::store::ScalarSettings::default()
            })
            .expect("saving scalars must work");
    }
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening must succeed");
        let scalars = reopened
            .load_settings()
            .expect("loading settings must work")
            .scalars;
        assert!(
            scalars.reduce_motion,
            "reduce_motion = true must survive the round trip"
        );
    }

    // And the opt-out round-trips too.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("reopening for the save must succeed");
        store
            .save_scalars(&riff_persistence::store::ScalarSettings {
                reduce_motion: false,
                ..riff_persistence::store::ScalarSettings::default()
            })
            .expect("saving scalars must work");
    }
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let scalars = reopened
        .load_settings()
        .expect("loading settings must work")
        .scalars;
    assert!(
        !scalars.reduce_motion,
        "reduce_motion = false must survive the round trip"
    );
}

/// Migration 012 retires the list/grid browser layout: the persisted scalar
/// column is dropped with the concept. Simulate a store that predates the
/// migration — un-apply 012 and re-add the column — then reopen through the
/// full migration path: the column must drop, a store that previously had
/// grid engaged opens on the now-permanent list, and every remaining scalar
/// carries over exactly.
#[test]
fn test_store_migration_012_drops_the_browser_layout_column() {
    // A fully migrated fresh store is the base: every scalar at defaults.
    let (_dir, db_path, store, _changes_rx) = open_store_with_path();

    // Roll back to the 011-era state: forget migration 012 and restore the
    // column exactly as migration 007 created it, plus a non-default grid
    // engagement and some non-default scalars that the rebuild must preserve.
    // The later app_settings-column migrations (013 `close_quits_app`, 015
    // the metadata version, 017 the ReplayGain Mode, 018 the pass's Settings
    // gating, 019 Reduce Motion) are also un-applied, because 012 rebuilds the
    // settings row from a column snapshot predating them; re-applying them
    // keeps the newest scalars present for reads.
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "DELETE FROM schema_migrations WHERE version IN (12, 13, 15, 17, 18, 19);
                 ALTER TABLE app_settings
                   ADD COLUMN browser_layout INTEGER NOT NULL DEFAULT 0
                     CHECK (browser_layout IN (0, 1));
                 UPDATE app_settings
                   SET browser_layout = 1, volume = 0.42, shuffle = 1,
                       smart_lists_collapsed = 1;",
            )
        })
        .expect("rolling back to the 011-era schema must work");
    drop(store);

    // Reopening applies only the pending migration 012: the table is rebuilt
    // without the column and the remaining scalars ride along.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let upgraded = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let columns: Vec<String> = upgraded
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('app_settings')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("reading the settings table's columns must work");
    assert!(
        !columns.iter().any(|column| column == "browser_layout"),
        "migration 012 must drop the browser-layout column"
    );

    let scalars = upgraded
        .load_settings()
        .expect("loading settings from the upgraded store must work")
        .scalars;
    assert_eq!(
        scalars.volume,
        Some(0.42),
        "the rebuild must preserve volume"
    );
    assert!(scalars.shuffle, "the rebuild must preserve shuffle");
    assert!(
        scalars.smart_lists_collapsed,
        "the rebuild must preserve smart-lists collapsed"
    );
}

/// A fresh store never contains the retired browser-layout column: the
/// migration set's newest schema is what a new install gets.
#[test]
fn test_store_fresh_start_never_creates_the_browser_layout_column() {
    let (_dir, store, _changes_rx) = open_store();

    let columns: Vec<String> = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('app_settings')")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("reading the settings table's columns must work");
    assert!(
        !columns.iter().any(|column| column == "browser_layout"),
        "a fresh store must never contain the retired browser-layout column"
    );
}

#[test]
fn test_store_library_paths_and_watch_states_roundtrip_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Change paths + watch states, then drop the connection ("restart").
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let mut watch_states = HashMap::new();
        watch_states.insert(PathBuf::from("music/a"), WatchState::Enabled);
        watch_states.insert(
            PathBuf::from("music/b"),
            WatchState::Warning("network mount".to_string()),
        );
        store
            .save_watch_states(&watch_states)
            .expect("saving watch states must work");
        store
            .save_library_paths(&[PathBuf::from("music/b"), PathBuf::from("music/a")])
            .expect("saving library paths must work");
    }

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let mut reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let settings = reopened
        .load_settings()
        .expect("loading settings must work");

    assert_eq!(
        settings.library_paths,
        vec![PathBuf::from("music/b"), PathBuf::from("music/a")],
        "registration order must survive the round trip"
    );
    assert_eq!(settings.watch_states.len(), 2);
    assert_eq!(
        settings.watch_states.get(&PathBuf::from("music/a")),
        Some(&WatchState::Enabled)
    );
    assert_eq!(
        settings.watch_states.get(&PathBuf::from("music/b")),
        Some(&WatchState::Warning("network mount".to_string()))
    );

    // Replacing the map removes stale entries: a path deleted from settings
    // must not keep its old watch row.
    let mut replacement = std::collections::HashMap::new();
    replacement.insert(PathBuf::from("music/c"), WatchState::Disabled);
    reopened.save_watch_states(&replacement).unwrap();
    reopened
        .save_library_paths(&[PathBuf::from("music/c")])
        .unwrap();

    let after = reopened.load_settings().unwrap();
    assert_eq!(after.library_paths, vec![PathBuf::from("music/c")]);
    assert_eq!(
        after.watch_states,
        std::collections::HashMap::from([(PathBuf::from("music/c"), WatchState::Disabled)])
    );
}

// --- Application Store: Library scan settings (design-handoff issue 12) ------

use riff_persistence::store::{AUDIO_EXTENSIONS, FullScanSummary};

#[test]
fn test_store_library_scan_settings_roundtrip_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Fresh store: the scanner defaults — hidden files skipped, every
    // supported format indexed, embedded artwork read.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let settings = store
            .load_settings()
            .expect("loading settings from a fresh store must work");
        assert!(settings.scalars.skip_hidden_files);
        assert_eq!(settings.scalars.scan_formats.len(), AUDIO_EXTENSIONS.len());
        assert!(settings.scalars.read_embedded_artwork);
    }

    // Change the library scan settings and drop the connection ("restart").
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let scalars = riff_persistence::store::ScalarSettings {
            skip_hidden_files: false,
            scan_formats: vec!["flac".to_string(), "mp3".to_string()],
            read_embedded_artwork: false,
            ..Default::default()
        };
        store
            .save_scalars(&scalars)
            .expect("saving scalars must work");
    }

    // Reopen: every library scan setting survives in its typed column.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let settings = reopened
        .load_settings()
        .expect("loading settings must work");
    assert!(!settings.scalars.skip_hidden_files);
    assert_eq!(
        settings.scalars.scan_formats,
        vec!["flac".to_string(), "mp3".to_string()]
    );
    assert!(!settings.scalars.read_embedded_artwork);
}

#[test]
fn test_store_last_full_scan_summary_roundtrip_across_reopen() {
    use riff_persistence::store::FullScanSummary;
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // A fresh store has never scanned.
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        assert!(store.last_full_scan().unwrap().is_none());
    }

    // Record one completed full scan with its counts and drop the
    // connection ("restart").
    let at = std::time::SystemTime::now();
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        store
            .record_full_scan_completed(FullScanSummary {
                at,
                files: 128,
                errors: 3,
            })
            .expect("recording the scan summary must work");
    }

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let summary = reopened
        .last_full_scan()
        .expect("reading the scan summary must work")
        .expect("the recorded scan survives the restart");
    assert_eq!(summary.files, 128);
    assert_eq!(summary.errors, 3);
    assert!(
        summary
            .at
            .duration_since(at - std::time::Duration::from_secs(1))
            .is_ok(),
        "the timestamp round-trips"
    );

    // A later scan overwrites the previous summary.
    let mut store = reopened;
    store
        .record_full_scan_completed(FullScanSummary {
            at,
            files: 200,
            errors: 0,
        })
        .unwrap();
    let summary = store.last_full_scan().unwrap().unwrap();
    assert_eq!(summary.files, 200);
    assert_eq!(summary.errors, 0);
}

// --- Application Store: Playlists, atomic per-mutation durability ------------

use riff_persistence::playlist::PlaylistId;
use riff_persistence::store::PlaylistStore;
use riff_persistence::track::TrackId;

#[test]
fn test_store_fresh_playlists_are_empty() {
    let (_dir, store, _changes_rx) = open_store();
    let playlists = store
        .load_playlists()
        .expect("loading playlists from a fresh store must work");
    assert!(playlists.is_empty(), "fresh store has no playlists");
}

#[test]
fn test_store_created_playlist_survives_immediate_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Create a playlist with initial entries; the mutation commits before
    // the connection is dropped (the "restart").
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let id = store
            .create_playlist(
                "  Chill Mix  ",
                &[TrackId("a.mp3".to_string()), TrackId("b.mp3".to_string())],
            )
            .expect("creating a playlist must work");
        assert_eq!(
            id.0,
            format!("chill-mix-{}", id.0.rsplit('-').next().unwrap())
        );
    }

    // Reopen: the playlist and its entry order are intact.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening must succeed");
    let playlists = reopened
        .load_playlists()
        .expect("loading playlists must work");
    assert_eq!(playlists.len(), 1, "the created playlist survives restart");
    assert_eq!(playlists[0].name, "Chill Mix", "name is trimmed");
    assert_eq!(
        playlists[0].tracks,
        vec![TrackId("a.mp3".to_string()), TrackId("b.mp3".to_string())],
        "initial entries keep their order"
    );
    assert!(playlists[0].created.is_some(), "creation time is stamped");
}

#[test]
fn test_store_rename_and_delete_commit_instantly_without_touching_unrelated_data() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let keep = store.create_playlist("Keep", &[]).unwrap();
        let gone = store
            .create_playlist("Gone", &[TrackId("g1.mp3".to_string())])
            .unwrap();

        // Rename commits instantly (no later save step exists to make it
        // durable) and reports the known id.
        let renamed = store
            .rename_playlist(&keep, "  Kept Forever  ")
            .expect("renaming must not error");
        assert!(renamed, "renaming a known playlist returns true");

        // Delete cascades the playlist's entries and reports the removal.
        let deleted = store
            .delete_playlist(&gone)
            .expect("deleting must not error");
        assert!(deleted, "deleting a known playlist returns true");

        // Unknown ids are no-ops that report false.
        let unknown = PlaylistId("never-created".to_string());
        assert!(!store.rename_playlist(&unknown, "X").unwrap());
        assert!(!store.delete_playlist(&unknown).unwrap());

        drop(store);

        // Simulated crash right after the actions: reopen and verify.
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let reopened =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let playlists = reopened.load_playlists().unwrap();
        assert_eq!(
            playlists
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Kept Forever"],
            "rename survived the restart; the deleted playlist is gone"
        );
        // Unrelated data was untouched: the surviving playlist has no stray
        // entries from the deleted one.
        assert!(
            playlists[0].tracks.is_empty(),
            "unrelated playlist keeps its own (empty) entry list"
        );
    }

    // The cascade removed the deleted playlist's entries at the SQL level.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let orphan_entries: i64 = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM playlist_entries WHERE track_id = 'g1.mp3'",
                [],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(
        orphan_entries, 0,
        "delete must remove the playlist's entries"
    );
}

#[test]
fn test_store_entry_add_remove_semantics_and_dangling_survival() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let pid;

    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        pid = store.create_playlist("P", &[]).unwrap();

        // Appends land at the end, in call order.
        assert!(
            store
                .add_playlist_entry(&pid, &TrackId("a.mp3".to_string()))
                .unwrap()
        );
        assert!(
            store
                .add_playlist_entry(&pid, &TrackId("b.mp3".to_string()))
                .unwrap()
        );
        // Exact duplicates are rejected.
        assert!(
            !store
                .add_playlist_entry(&pid, &TrackId("a.mp3".to_string()))
                .unwrap()
        );
        // Unknown playlist ids are no-ops.
        assert!(
            !store
                .add_playlist_entry(&PlaylistId("x".to_string()), &TrackId("c.mp3".to_string()))
                .unwrap()
        );

        // Removal reports whether anything was removed.
        assert!(
            store
                .remove_playlist_entries(&pid, &TrackId("a.mp3".to_string()))
                .unwrap()
        );
        assert!(
            !store
                .remove_playlist_entries(&pid, &TrackId("zzz.mp3".to_string()))
                .unwrap()
        );

        // A dangling reference (no tracks table exists at all) commits like
        // any other entry.
        assert!(
            store
                .add_playlist_entry(&pid, &TrackId("vanished\\file.flac".to_string()))
                .unwrap()
        );
        drop(store);
    }

    // Restart: ordering and the dangling entry both survive; the entry stays
    // listed so it resolves again once the referenced file returns.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let playlists = reopened.load_playlists().unwrap();
    assert_eq!(playlists.len(), 1);
    assert_eq!(
        playlists[0].tracks,
        vec![
            TrackId("b.mp3".to_string()),
            TrackId("vanished\\file.flac".to_string())
        ],
        "entry order preserved; dangling reference kept visible"
    );
}

#[test]
fn test_store_distinct_names_that_slug_alike_still_get_unique_ids() {
    let (_dir, mut store, _changes_rx) = open_store();
    // Names are unique across the sidebar now (ticket 03), so the id-dedup
    // loop can no longer be reached by two rows sharing a name. It is still
    // reachable: two *different* names can slug alike inside one millisecond,
    // and then the suffixing loop is what keeps their ids apart.
    let a = store.create_playlist("Mix A", &[]).unwrap();
    let b = store.create_playlist("Mix.A", &[]).unwrap();
    assert_ne!(a, b, "ids must be unique even for slugs that agree");
    let playlists = store.load_playlists().unwrap();
    assert_eq!(playlists.len(), 2);
    assert_eq!(playlists[0].name, "Mix A", "each row keeps its own name");
    assert_eq!(playlists[1].name, "Mix.A", "unchanged by the id suffixing");
}

// --- Application Store: playlist drag-reorder persistence (Issue 12) --------

#[test]
fn test_store_reorder_playlist_entries_persists_the_new_order_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let pid;

    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        pid = store
            .create_playlist(
                "Gym",
                &[
                    TrackId("a.mp3".to_string()),
                    TrackId("b.mp3".to_string()),
                    TrackId("c.mp3".to_string()),
                ],
            )
            .unwrap();

        // Drag the first entry down between the others: A,B,C → B,A,C.
        // One immediate durable transaction rewrites the positions.
        let reordered = store
            .reorder_playlist_entries(
                &pid,
                &[
                    TrackId("b.mp3".to_string()),
                    TrackId("a.mp3".to_string()),
                    TrackId("c.mp3".to_string()),
                ],
            )
            .expect("reordering a known playlist must work");
        assert!(reordered, "reordering a known playlist returns true");
        drop(store);
    }

    // Reopen: the new order survived the restart.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let playlists = reopened.load_playlists().unwrap();
    assert_eq!(
        playlists[0].tracks,
        vec![
            TrackId("b.mp3".to_string()),
            TrackId("a.mp3".to_string()),
            TrackId("c.mp3".to_string())
        ],
        "the dragged order persisted through the PlaylistStore"
    );

    // Reordering again (C first) keeps working on the persisted data.
    let mut store = reopened;
    assert!(
        store
            .reorder_playlist_entries(
                &pid,
                &[
                    TrackId("c.mp3".to_string()),
                    TrackId("b.mp3".to_string()),
                    TrackId("a.mp3".to_string())
                ]
            )
            .unwrap()
    );
    assert_eq!(
        store.load_playlists().unwrap()[0].tracks[0],
        TrackId("c.mp3".to_string()),
        "re-reordering rewrites the persisted order"
    );
}

#[test]
fn test_store_reorder_unknown_playlist_is_a_noop_and_other_playlists_are_untouched() {
    let (_dir, mut store, _changes_rx) = open_store();
    let keep = store
        .create_playlist(
            "Keep",
            &[TrackId("k1.mp3".to_string()), TrackId("k2.mp3".to_string())],
        )
        .unwrap();

    let unknown = PlaylistId("never-created".to_string());
    let reordered = store
        .reorder_playlist_entries(&unknown, &[TrackId("x.mp3".to_string())])
        .expect("reordering an unknown playlist must not error");
    assert!(!reordered, "unknown ids report false");

    // The unrelated playlist's entries are untouched.
    let playlists = store.load_playlists().unwrap();
    assert_eq!(
        playlists[0].tracks,
        vec![TrackId("k1.mp3".to_string()), TrackId("k2.mp3".to_string())],
        "an unknown-id reorder never touches other playlists"
    );
    let _ = keep;
}

#[test]
fn test_store_playlist_entries_report_library_validity_via_left_join() {
    let (_dir, _, mut store, _changes_rx) = open_store_with_path();

    // Two scanned tracks plus a dangling reference that was never scanned.
    store
        .apply_scan_batch(&[
            library_track("m:\\lib\\a.mp3", "A", None, "One", None),
            library_track("m:\\lib\\b.mp3", "B", None, "One", None),
        ])
        .expect("batch applies");
    let pid = store
        .create_playlist(
            "Mix",
            &[
                TrackId("m:\\lib\\a.mp3".to_string()),
                TrackId("m:\\gone\\deleted.mp3".to_string()),
                TrackId("m:\\lib\\b.mp3".to_string()),
            ],
        )
        .unwrap();

    // Entries come back in playlist order; known tracks ride their full
    // row (valid=true), the dangling reference stays listed with its track
    // unset and valid=false — product behavior, never dropped.
    let entries = store.load_playlist_entries(&pid).expect("entries load");
    assert_eq!(entries.len(), 3, "dangling references stay listed");
    assert_eq!(entries[0].id.0, "m:\\lib\\a.mp3");
    assert!(entries[0].valid, "a scanned track is Library-valid");
    let track = entries[0].track.as_ref().expect("known track resolves");
    assert_eq!(track.metadata.title.as_deref(), Some("A"));
    assert_eq!(track.file_path, PathBuf::from("m:\\lib\\a.mp3"));
    assert!(!entries[1].valid, "dangling reference is flagged invalid");
    assert!(
        entries[1].track.is_none(),
        "dangling reference has no track"
    );
    assert_eq!(entries[1].id.0, "m:\\gone\\deleted.mp3");
    assert!(entries[2].valid);
    assert_eq!(entries[2].id.0, "m:\\lib\\b.mp3");

    // An unknown playlist id yields an empty entry list.
    let unknown = PlaylistId("never-created".to_string());
    assert!(
        store
            .load_playlist_entries(&unknown)
            .expect("unknown playlist loads")
            .is_empty()
    );

    // Validity is read-time: removing a track from the Library flips its
    // entry to invalid on the next query while the entry itself survives.
    store
        .remove_library_path(std::path::Path::new("m:\\lib\\a.mp3"))
        .expect("removal works");
    let entries = store.load_playlist_entries(&pid).expect("reload works");
    assert_eq!(entries.len(), 3, "the entry is not silently dropped");
    assert!(
        !entries[0].valid && entries[0].track.is_none(),
        "a track removed from the Library turns its entry dangling"
    );
}

// --- Application Store: Library collection (ticket 05) ---------------------

use riff_persistence::store::{LibraryMutationStore, LibraryQueryStore, Page, TrackListOrder};
use std::time::Duration;

/// Build a Track fixture with full metadata control (compilation cases need
/// track-level `artist` distinct from `album_artist`).
fn library_track(
    path: &str,
    title: &str,
    artist: Option<&str>,
    album: &str,
    album_artist: Option<&str>,
) -> Track {
    Track::fresh_scan(
        TrackId::from_path(std::path::Path::new(path)),
        PathBuf::from(path),
        TrackMetadata {
            title: Some(title.to_string()),
            artist: artist.map(str::to_string),
            album: Some(album.to_string()),
            album_artist: album_artist.map(str::to_string),
            ..TrackMetadata::default()
        },
        None,
        None,
        None,
        None,
    )
}

// --- Deleted window + count reads, in terms of Listing Pages ---------------
//
// The `LibraryQueryStore` port collapsed every windowed read and its count
// read into one Listing Page: a `Page<T>` carries the window AND the total,
// read under a single connection acquisition. These helpers express each
// deleted read through the page that replaced it, so the tests below keep
// asserting exactly the facts they asserted before. A `*_window` helper reads
// the page's rows; a `*_count` helper reads the page's total, asking for a
// one-row window since only the total is wanted. The browse listings' pages
// take a `SortDirection` their count reads never had; a total is
// direction-independent in the store, so the count helpers pass `Ascending`.

/// A page read's rows, unwrapped — the `*_window` helpers' shared body.
fn page_rows<T: Clone>(page: Result<Page<T>, StoreError>, what: &str) -> Vec<T> {
    page.expect(what).rows().to_vec()
}

/// A page read's total, unwrapped — the `*_count` helpers' shared body.
fn page_total<T>(page: Result<Page<T>, StoreError>, what: &str) -> usize {
    page.expect(what).total()
}

/// A fresh, fully migrated store in a scratch directory — the fixture most
/// store tests start from. Returns the directory guard (keep it alive: it
/// owns the database file), the store, and the change receiver for tests
/// that assert on the event backbone.
fn open_store() -> (
    tempfile::TempDir,
    SqliteStore,
    crossbeam_channel::Receiver<StoreChanged>,
) {
    let (dir, _, store, changes_rx) = open_store_with_path();
    (dir, store, changes_rx)
}

/// [`open_store`] plus the database path the open actually used — for tests
/// that need the file itself: reopening the same path on a later handle,
/// reading or rewriting it, or deriving sibling paths beside it. The path
/// comes from the same place the open took it, so callers never re-spell the
/// store filename.
fn open_store_with_path() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    SqliteStore,
    crossbeam_channel::Receiver<StoreChanged>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (changes_tx, changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let store = SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    (dir, db_path, store, changes_rx)
}

/// The flat library listing's window, from its Listing Page.
fn tracks_window(store: &SqliteStore, offset: usize, limit: usize) -> Vec<Track> {
    page_rows(
        store.tracks_page(TrackListOrder::default(), offset, limit),
        "tracks page reads",
    )
}

/// The flat library listing's total, from its Listing Page.
fn track_count(store: &SqliteStore) -> usize {
    page_total(
        store.tracks_page(TrackListOrder::default(), 0, 1),
        "tracks page reads",
    )
}

/// The search listing's window, from its Listing Page.
fn search_window(store: &SqliteStore, query: &str, offset: usize, limit: usize) -> Vec<Track> {
    page_rows(
        store.search_page(query, TrackListOrder::default(), offset, limit),
        "search page reads",
    )
}

/// The search listing's total, from its Listing Page.
fn search_count(store: &SqliteStore, query: &str) -> usize {
    page_total(
        store.search_page(query, TrackListOrder::default(), 0, 1),
        "search page reads",
    )
}

/// The hit-album listing's window, from its Listing Page.
fn hit_albums(store: &SqliteStore, query: &str, offset: usize, limit: usize) -> Vec<Album> {
    page_rows(
        store.hit_albums_page(query, offset, limit),
        "hit albums page reads",
    )
}

/// The hit-album listing's total, from its Listing Page.
fn hit_albums_count(store: &SqliteStore, query: &str) -> usize {
    page_total(store.hit_albums_page(query, 0, 1), "hit albums page reads")
}

/// The hit-artist listing's window, from its Listing Page.
fn hit_artists(store: &SqliteStore, query: &str, offset: usize, limit: usize) -> Vec<Artist> {
    page_rows(
        store.hit_artists_page(query, offset, limit),
        "hit artists page reads",
    )
}

/// The hit-artist listing's total, from its Listing Page.
fn hit_artists_count(store: &SqliteStore, query: &str) -> usize {
    page_total(
        store.hit_artists_page(query, 0, 1),
        "hit artists page reads",
    )
}

/// The Artists root's window, from its Listing Page.
fn artists_window(
    store: &SqliteStore,
    direction: SortDirection,
    offset: usize,
    limit: usize,
) -> Vec<Artist> {
    page_rows(
        store.artists_page(direction, offset, limit),
        "artists page reads",
    )
}

/// The Artists root's total, from its Listing Page.
fn artists_count(store: &SqliteStore) -> usize {
    page_total(
        store.artists_page(SortDirection::Ascending, 0, 1),
        "artists page reads",
    )
}

/// The Albums root's window, from its Listing Page.
fn albums_window(
    store: &SqliteStore,
    direction: SortDirection,
    offset: usize,
    limit: usize,
) -> Vec<Album> {
    page_rows(
        store.albums_page(direction, offset, limit),
        "albums page reads",
    )
}

/// The Albums root's total, from its Listing Page.
fn albums_count(store: &SqliteStore) -> usize {
    page_total(
        store.albums_page(SortDirection::Ascending, 0, 1),
        "albums page reads",
    )
}

/// The Genres root's window, from its Listing Page.
fn genres_window(
    store: &SqliteStore,
    direction: SortDirection,
    offset: usize,
    limit: usize,
) -> Vec<GenreCount> {
    page_rows(
        store.genres_page(direction, offset, limit),
        "genres page reads",
    )
}

/// The Genres root's total, from its Listing Page.
fn genres_count(store: &SqliteStore) -> usize {
    page_total(
        store.genres_page(SortDirection::Ascending, 0, 1),
        "genres page reads",
    )
}

/// The genre drill-down's artist window, from its Listing Page.
fn artists_in_genre_window(
    store: &SqliteStore,
    genre: &str,
    direction: SortDirection,
    offset: usize,
    limit: usize,
) -> Vec<Artist> {
    page_rows(
        store.artists_in_genre_page(genre, direction, offset, limit),
        "artists in genre page reads",
    )
}

/// The genre drill-down's artist total, from its Listing Page.
fn artists_in_genre_count(store: &SqliteStore, genre: &str) -> usize {
    page_total(
        store.artists_in_genre_page(genre, SortDirection::Ascending, 0, 1),
        "artists in genre page reads",
    )
}

/// The artist-and-genre drill-down's album window, from its Listing Page.
fn artist_albums_in_genre_window(
    store: &SqliteStore,
    artist: &str,
    genre: &str,
    direction: SortDirection,
    offset: usize,
    limit: usize,
) -> Vec<Album> {
    page_rows(
        store.artist_albums_in_genre_page(artist, genre, direction, offset, limit),
        "artist albums in genre page reads",
    )
}

/// The artist-and-genre drill-down's album total, from its Listing Page.
fn artist_albums_in_genre_count(store: &SqliteStore, artist: &str, genre: &str) -> usize {
    page_total(
        store.artist_albums_in_genre_page(artist, genre, SortDirection::Ascending, 0, 1),
        "artist albums in genre page reads",
    )
}

#[test]
fn test_store_library_migration_004_applies_and_reopens_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("fresh store must open and migrate");
    let tables: Vec<String> = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table'
                 AND name IN ('artists', 'albums', 'tracks') ORDER BY name",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("reading table list must work");
    assert_eq!(
        tables,
        ["albums", "artists", "tracks"],
        "library collection tables must exist after migration"
    );
    drop(store);

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening a store with the library schema must succeed");
}

#[test]
fn test_scan_batch_populates_collection_including_compilations() {
    let (_dir, mut store, _changes_rx) = open_store();

    // Compilation: one album credited to "Various Artists", two tracks with
    // their own track-level artists.
    let batch = vec![
        library_track(
            "m:\\music\\comp\\01.mp3",
            "Song One",
            Some("Artist A"),
            "Comp",
            Some("Various Artists"),
        ),
        library_track(
            "m:\\music\\comp\\02.mp3",
            "Song Two",
            Some("Artist B"),
            "Comp",
            Some("Various Artists"),
        ),
    ];
    let written = store.apply_scan_batch(&batch).expect("batch must apply");
    assert_eq!(written, 2, "both tracks of the batch are written");

    let (artists, albums): (i64, i64) = store
        .with_connection(|conn| {
            let artists: i64 =
                conn.query_row("SELECT COUNT(*) FROM artists", [], |row| row.get(0))?;
            let albums: i64 =
                conn.query_row("SELECT COUNT(*) FROM albums", [], |row| row.get(0))?;
            Ok((artists, albums))
        })
        .expect("counting parents must work");
    assert_eq!(artists, 1, "one album-artist grouping for the compilation");
    assert_eq!(
        albums, 1,
        "one album row identified by (album_artist, title)"
    );

    let rows: Vec<(String, Option<String>, String, String, String)> = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path, artist, album_artist_key, album_title_key, search_text
                 FROM tracks ORDER BY path",
            )?;
            let mapped = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?;
            mapped.collect()
        })
        .expect("reading track rows must work");

    assert_eq!(rows.len(), 2);
    // Track-level artists stay distinct per track...
    assert_eq!(rows[0].1.as_deref(), Some("Artist A"));
    assert_eq!(rows[1].1.as_deref(), Some("Artist B"));
    // ...while the album identity keys use the resolved display fallbacks.
    for row in &rows {
        assert_eq!(row.2, "Various Artists");
        assert_eq!(row.3, "Comp");
    }
    // Derived search text is the Rust-lowercased concatenation, exactly like
    // TrackMetadata::search_text().
    assert_eq!(
        rows[0].4,
        batch[0].metadata.search_text(),
        "search_text must be derived in Rust at write time"
    );
}

#[test]
fn test_interrupted_scan_keeps_committed_batches() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    // Batch 1 commits; then the process "dies" before batch 2 is ever sent
    // (the store handle is dropped without applying more batches).
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let batch1 = vec![library_track(
            "m:\\music\\a.mp3",
            "A",
            Some("Artist A"),
            "Album A",
            None,
        )];
        store
            .apply_scan_batch(&batch1)
            .expect("batch 1 must commit");
    }

    // After reopening, everything committed so far is present.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let count: i64 = reopened
        .with_connection(|conn| conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0)))
        .expect("counting tracks must work");
    assert_eq!(count, 1, "committed scan progress survives an interruption");
}

#[test]
fn test_rescan_is_idempotent_and_preserves_history() {
    let (_dir, mut store, _changes_rx) = open_store();

    let first = vec![library_track(
        "m:\\music\\old.mp3",
        "Old Title",
        Some("Artist A"),
        "Album A",
        None,
    )];
    store
        .apply_scan_batch(&first)
        .expect("initial scan applies");

    // Simulate accumulated play history on the stored row.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 3,
                 last_played_nanos = 1000, date_added_nanos = 500
                 WHERE path = 'm:\\music\\old.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    // Rescan the same path with refreshed metadata.
    let rescanned = vec![library_track(
        "m:\\music\\old.mp3",
        "New Title",
        Some("Artist A"),
        "Album A",
        None,
    )];
    store.apply_scan_batch(&rescanned).expect("rescan applies");

    let (tracks, albums, artists): (i64, i64, i64) = store
        .with_connection(|conn| {
            let t: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
            let al: i64 = conn.query_row("SELECT COUNT(*) FROM albums", [], |r| r.get(0))?;
            let ar: i64 = conn.query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0))?;
            Ok((t, al, ar))
        })
        .expect("counting rows must work");
    assert_eq!(
        (tracks, albums, artists),
        (1, 1, 1),
        "rescan must not duplicate rows"
    );

    let (title, play_count, last_played, date_added): (String, i64, Option<i64>, Option<i64>) =
        store
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT title, play_count, last_played_nanos, date_added_nanos
                     FROM tracks WHERE path = 'm:\\music\\old.mp3'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .expect("reading the upserted row must work");
    assert_eq!(title, "New Title", "metadata refreshes on rescan");
    assert_eq!(play_count, 3, "play history must survive a rescan");
    assert_eq!(last_played, Some(1000), "last-played must survive a rescan");
    assert_eq!(date_added, Some(500), "date-added must survive a rescan");
}

#[test]
fn test_rescan_refreshes_replaygain_pair_after_external_retag() {
    let (_dir, mut store, _changes_rx) = open_store();

    // First scan: the file carries an older measurement.
    let mut first = library_track(
        "m:\\music\\old.mp3",
        "Old Title",
        Some("Artist A"),
        "Album A",
        None,
    );
    first.metadata.replaygain_track_gain = Some(-6.54);
    first.metadata.replaygain_track_peak = Some(0.75);
    first.metadata.replaygain_album_gain = Some(-7.12);
    first.metadata.replaygain_album_peak = Some(0.81);
    store
        .apply_scan_batch(std::slice::from_ref(&first))
        .expect("initial scan applies");

    // The file is re-tagged by an external tool and the rescan re-reads it:
    // the stored values refresh in place rather than duplicating or going
    // stale.
    let mut rescanned = library_track(
        "m:\\music\\old.mp3",
        "Old Title",
        Some("Artist A"),
        "Album A",
        None,
    );
    rescanned.metadata.replaygain_track_gain = Some(-9.20);
    rescanned.metadata.replaygain_track_peak = Some(0.90);
    rescanned.metadata.replaygain_album_gain = Some(-8.00);
    rescanned.metadata.replaygain_album_peak = Some(0.95);
    store
        .apply_scan_batch(std::slice::from_ref(&rescanned))
        .expect("rescan applies");

    let got = store
        .get_track(&first.id)
        .expect("get_track works")
        .expect("known id resolves");
    assert_eq!(got.metadata.replaygain_track_gain, Some(-9.20));
    assert_eq!(got.metadata.replaygain_track_peak, Some(0.90));
    assert_eq!(got.metadata.replaygain_album_gain, Some(-8.00));
    assert_eq!(got.metadata.replaygain_album_peak, Some(0.95));

    let (tracks, albums): (i64, i64) = store
        .with_connection(|conn| {
            let t: i64 = conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
            let al: i64 = conn.query_row("SELECT COUNT(*) FROM albums", [], |r| r.get(0))?;
            Ok((t, al))
        })
        .expect("counting rows must work");
    assert_eq!((tracks, albums), (1, 1), "rescan must not duplicate rows");
}

#[test]
fn test_foreign_keys_reject_orphan_tracks() {
    let (_dir, store, _changes_rx) = open_store();

    let err = store
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO tracks(path, search_text, album_artist_key, album_title_key)
                 VALUES ('x.mp3', 'x', 'Ghost Artist', 'Ghost Album')",
                [],
            )
        })
        .expect_err("a track without its album parent must violate the foreign key");
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "error must name the FK violation: {err}"
    );
}

#[test]
fn test_flat_list_windows_are_path_ordered_and_bounded() {
    let (_dir, mut store, _changes_rx) = open_store();

    // Shuffled insertion order with byte-ordering traps: uppercase sorts
    // before lowercase ('B' 0x42 < 'a' 0x61), digits before letters.
    let paths = [
        "m:\\z.mp3",
        "M:\\A\\B.mp3",
        "m:\\a.mp3",
        "m:\\1.mp3",
        "M:\\A\\a.mp3",
    ];
    let batch: Vec<Track> = paths
        .iter()
        .map(|p| library_track(p, "T", Some("Artist"), "Album", None))
        .collect();
    store.apply_scan_batch(&batch).expect("batch applies");

    let mut expected: Vec<String> = paths.iter().map(std::string::ToString::to_string).collect();
    expected.sort(); // Rust byte-wise sort = SQLite BINARY collation

    assert_eq!(track_count(&store), 5);

    let first_page = tracks_window(&store, 0, 3);
    assert_eq!(
        first_page
            .iter()
            .map(|t| t.id.0.clone())
            .collect::<Vec<_>>(),
        expected[..3],
        "flat list must be deterministically path-ordered"
    );
    let tail = tracks_window(&store, 3, 10);
    assert_eq!(
        tail.iter().map(|t| t.id.0.clone()).collect::<Vec<_>>(),
        expected[3..],
        "window past the page must return the remaining rows"
    );
    assert!(
        tracks_window(&store, 100, 5).is_empty(),
        "offset past the end yields an empty window"
    );
}

#[test]
fn test_flat_list_orders_land_in_sql_per_track_list_order() {
    use riff_persistence::store::TrackListOrder as Order;

    let (_dir, mut store, _changes_rx) = open_store();

    // Path order deliberately disagrees with title order, and titles carry
    // a case trap: NOCASE folding must rank 'b' beside 'A', while byte-wise
    // ordering would not.
    let batch: Vec<Track> = [
        ("m:\\2.mp3", "b-side"),
        ("m:\\1.mp3", "Anchor"),
        ("m:\\3.mp3", "Cascade"),
    ]
    .iter()
    .map(|(p, t)| library_track(p, t, Some("Artist"), "Album", None))
    .collect();
    store.apply_scan_batch(&batch).expect("batch applies");

    let ordered = |order: TrackListOrder| {
        store
            .tracks_page(order, 0, 10)
            .expect("tracks page reads")
            .rows()
            .iter()
            .map(|t| t.metadata.title.clone().unwrap())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        ordered(Order::PathAsc),
        ["Anchor", "b-side", "Cascade"],
        "path-ascending is the canonical flat order (path order, not title order)"
    );
    assert_eq!(
        ordered(Order::PathDesc),
        ["Cascade", "b-side", "Anchor"],
        "path-descending is the exact SQL reversal"
    );
    // NOCASE folds 'b' before 'C', where byte-wise order would not — the
    // asc list is therefore distinct from both the path order and a BINARY
    // title order, which is the trap the fixture sets.
    assert_eq!(
        ordered(Order::TitleAsc),
        ["Anchor", "b-side", "Cascade"],
        "title order is case-insensitive, path as the tiebreak"
    );
    assert_eq!(
        ordered(Order::TitleDesc),
        ["Cascade", "b-side", "Anchor"],
        "title-descending is the exact SQL reversal, path as the tiebreak"
    );
}

#[test]
fn test_all_track_ids_are_canonically_path_ordered() {
    let (_dir, db_path, store, _changes_rx) = open_store_with_path();

    // A fresh store has no ids to fill a queue with.
    assert_eq!(
        store.all_track_ids().expect("empty store lists no ids"),
        [] as [TrackId; 0]
    );

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let mut store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    // Shuffled insertion order with byte-ordering traps: Queue Fill must see
    // the canonical flat ordering (path ascending, ADR 0003), not insertion
    // or HashMap luck.
    let paths = [
        "m:\\z.mp3",
        "M:\\A\\B.mp3",
        "m:\\a.mp3",
        "m:\\1.mp3",
        "M:\\A\\a.mp3",
    ];
    let batch: Vec<Track> = paths
        .iter()
        .map(|p| library_track(p, "T", Some("Artist"), "Album", None))
        .collect();
    store.apply_scan_batch(&batch).expect("batch applies");

    let mut expected: Vec<String> = paths.iter().map(std::string::ToString::to_string).collect();
    expected.sort(); // Rust byte-wise sort = SQLite BINARY collation

    assert_eq!(
        store
            .all_track_ids()
            .expect("ids list works")
            .iter()
            .map(|id| id.0.clone())
            .collect::<Vec<_>>(),
        expected,
        "Queue Fill's data source is deterministically path-ordered"
    );
}

#[test]
fn test_search_parity_with_legacy_semantics() {
    let (_dir, mut store, _changes_rx) = open_store();

    // Fixture library including non-Latin titles and literal wildcard
    // characters in metadata.
    let fixtures = vec![
        library_track(
            "f:\\s1.mp3",
            "Summer Breeze",
            Some("The Beach Boys"),
            "Sun",
            None,
        ),
        library_track(
            "f:\\s2.mp3",
            "Лебединое озеро",
            Some("Чайковский"),
            "Балеты",
            None,
        ),
        library_track("f:\\s3.mp3", "東京物語", None, "小津安二郎", None),
        library_track(
            "f:\\s4.mp3",
            "100% Live_Recording",
            Some("Idol"),
            "Stage",
            Some("Various Artists"),
        ),
    ];
    store.apply_scan_batch(&fixtures).expect("batch applies");

    // The legacy in-memory semantics are the oracle: case-insensitive
    // substring over the derived search text.
    let expected_for = |query: &str| -> Vec<String> {
        let q = query.to_lowercase();
        let mut hits: Vec<String> = fixtures
            .iter()
            .filter(|t| t.metadata.search_text().contains(&q))
            .map(|t| t.id.0.clone())
            .collect();
        hits.sort();
        hits
    };

    for query in [
        "summer",
        "SUMMER",
        "чайковский",
        "ЧАЙКОВСКИЙ",
        "東京",
        "100%",
        "_rec",
        "",
        "various artists",
        "zzz-no-match",
    ] {
        let expected = expected_for(query);
        let got_count = search_count(&store, query);
        assert_eq!(got_count, expected.len(), "count parity for {query:?}");
        let got = search_window(&store, query, 0, 100);
        assert_eq!(
            got.iter().map(|t| t.id.0.clone()).collect::<Vec<_>>(),
            expected,
            "window content parity for {query:?}"
        );
    }

    // Bounded windows slice the match set deterministically.
    let all = search_window(&store, "", 0, 2);
    assert_eq!(all.len(), 2, "limit bounds the window");
    let rest = search_window(&store, "", 2, 50);
    assert_eq!(rest.len(), fixtures.len() - 2);
}

#[test]
fn test_get_track_roundtrips_all_fields() {
    let (_dir, mut store, _changes_rx) = open_store();

    let full = Track {
        id: TrackId::from_path(std::path::Path::new("f:\\full.flac")),
        file_path: PathBuf::from("f:\\full.flac"),
        metadata: TrackMetadata {
            title: Some("Full".to_string()),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: Some("Album Artist".to_string()),
            track_number: Some(7),
            disc_number: Some(2),
            genre: Some("Rock".to_string()),
            year: Some(1999),
            composer: Some("Composer".to_string()),
            comment: Some("Comment".to_string()),
            replaygain_track_gain: Some(-6.54),
            replaygain_track_peak: Some(0.75),
            replaygain_album_gain: Some(-7.12),
            replaygain_album_peak: Some(0.81),
        },
        duration: Some(Duration::from_secs_f32(12.5)),
        sample_rate: Some(44_100),
        channels: Some(2),
        play_count: 4,
        last_played: Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)),
        date_added: Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(500)),
        favorite: false,
        search_text: String::new(),
    };
    store
        .apply_scan_batch(std::slice::from_ref(&full))
        .expect("batch applies");
    // History columns are insert-only for scans; seed them directly to
    // exercise the read path.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 4,
                 last_played_nanos = 1000000000000, date_added_nanos = 500000000000
                 WHERE path = 'f:\\full.flac'",
                [],
            )
        })
        .expect("seeding history works");

    let got = store
        .get_track(&full.id)
        .expect("get_track works")
        .expect("known id resolves");
    assert_eq!(got.id, full.id);
    assert_eq!(got.file_path, full.file_path);
    assert_eq!(got.metadata, full.metadata, "metadata round-trips exactly");
    assert_eq!(got.duration, full.duration);
    assert_eq!(got.sample_rate, full.sample_rate);
    assert_eq!(got.channels, full.channels);
    assert_eq!(got.play_count, 4);
    assert_eq!(
        got.last_played,
        Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000))
    );
    assert_eq!(
        got.date_added,
        Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(500))
    );

    // Minimal track: every optional field absent.
    let minimal = library_track("f:\\min.mp3", "Min", None, "Album", None);
    let minimal = Track {
        metadata: TrackMetadata {
            title: None,
            artist: None,
            album: None,
            ..minimal.metadata
        },
        ..minimal
    };
    store
        .apply_scan_batch(std::slice::from_ref(&minimal))
        .expect("batch applies");
    let got_min = store
        .get_track(&minimal.id)
        .expect("get_track works")
        .expect("known id resolves");
    assert_eq!(got_min.metadata.title, None);
    assert_eq!(got_min.metadata.artist, None);
    assert_eq!(got_min.metadata.album, None);
    assert_eq!(
        got_min.metadata.replaygain_album_gain, None,
        "a track scanned without the tag stores NULL, not a default gain"
    );
    assert_eq!(
        got_min.metadata.replaygain_album_peak, None,
        "a track scanned without the tag stores NULL, not a default peak"
    );
    assert_eq!(got_min.duration, None);
    assert_eq!(got_min.last_played, None);

    let unknown = store
        .get_track(&TrackId("f:\\missing.mp3".to_string()))
        .expect("get_track works");
    assert!(unknown.is_none(), "unknown ids resolve to None");
}

// --- Application Store: collection event transactions (ticket 06) -----------

use std::time::SystemTime;

#[test]
fn test_record_track_played_persists_immediately_across_reopen() {
    let (_dir, db_path, mut store, _changes_rx) = open_store_with_path();

    let batch = vec![
        library_track("m:\\music\\a.mp3", "A", Some("Artist A"), "Album A", None),
        library_track("m:\\music\\b.mp3", "B", Some("Artist A"), "Album A", None),
    ];
    store.apply_scan_batch(&batch).expect("batch applies");

    let id_a = TrackId("m:\\music\\a.mp3".to_string());
    let first_play = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let second_play = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000);
    assert!(
        store
            .record_track_played(&id_a, first_play)
            .expect("recording a known track works"),
        "known track must report true"
    );
    assert!(
        store
            .record_track_played(&id_a, second_play)
            .expect("recording a known track works"),
        "second play must also record"
    );

    // Unknown ids are not an error but change nothing.
    assert!(
        !store
            .record_track_played(&TrackId("m:\\missing.mp3".to_string()), first_play)
            .expect("unknown id is not an error"),
        "unknown track must report false"
    );

    // Durability: the plays survive closing and reopening the store.
    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let a = reopened
        .get_track(&id_a)
        .expect("get_track works")
        .expect("track still known after reopen");
    assert_eq!(a.play_count, 2, "both finished plays persisted");
    assert_eq!(a.last_played, Some(second_play), "the last play wins");

    let b = reopened
        .get_track(&TrackId("m:\\music\\b.mp3".to_string()))
        .expect("get_track works")
        .expect("track still known after reopen");
    assert_eq!(b.play_count, 0, "other tracks are untouched");
    assert_eq!(b.last_played, None);
}

#[test]
fn test_tag_refresh_preserves_history_and_updates_album_derivation() {
    let (_dir, db_path, mut store, _changes_rx) = open_store_with_path();

    // Track A carries the album's derivation (first-added); B is newer.
    let mut track_a = library_track(
        "f:\\al\\a.mp3",
        "Old Title",
        Some("Artist A"),
        "Album A",
        None,
    );
    track_a.metadata.year = Some(2001);
    track_a.metadata.genre = Some("Rock".to_string());
    let mut track_b = library_track("f:\\al\\b.mp3", "B", Some("Artist A"), "Album A", None);
    track_b.metadata.year = Some(1999);
    track_b.metadata.genre = Some("Jazz".to_string());
    store
        .apply_scan_batch(&[track_a.clone(), track_b.clone()])
        .expect("batch applies");

    // Accumulated history on A (and its earlier date_added makes it the
    // album's first-added track).
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 3,
                 last_played_nanos = 1000, date_added_nanos = 500
                 WHERE path = 'f:\\al\\a.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    // Tag edit: title/year/genre change, everything else carried over —
    // exactly what `TagEdit::apply_to` produces before handing us the Track.
    let mut refreshed = track_a;
    refreshed.metadata.title = Some("New Title".to_string());
    refreshed.metadata.year = Some(2010);
    refreshed.metadata.genre = Some("Metal".to_string());
    store
        .apply_tag_refresh(&refreshed)
        .expect("refresh applies");

    // Durability: assert against a freshly opened store.
    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let a = reopened
        .get_track(&refreshed.id)
        .expect("get_track works")
        .expect("track known after reopen");
    assert_eq!(a.metadata.title.as_deref(), Some("New Title"));
    assert_eq!(a.metadata.year, Some(2010), "edited year lands");
    assert_eq!(a.metadata.genre.as_deref(), Some("Metal"));
    assert_eq!(
        a.metadata.artist.as_deref(),
        Some("Artist A"),
        "unchanged field stays"
    );
    assert_eq!(a.play_count, 3, "play history survives the tag refresh");
    assert_eq!(
        a.last_played,
        Some(SystemTime::UNIX_EPOCH + Duration::from_micros(1))
    );
    assert_eq!(
        a.date_added,
        Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(500))
    );

    let b = reopened
        .get_track(&track_b.id)
        .expect("get_track works")
        .expect("track known after reopen");
    assert_eq!(b.metadata.year, Some(1999), "other tracks untouched");

    // The album's derivation followed its first-added track's edit.
    let (album_year, album_genre): (Option<i64>, Option<String>) = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT year, genre FROM albums WHERE album_artist = 'Artist A' AND title = 'Album A'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("reading the album row must work");
    assert_eq!(
        album_year,
        Some(2010),
        "album year re-derived from the edit"
    );
    assert_eq!(album_genre.as_deref(), Some("Metal"));
}

#[test]
fn test_tag_refresh_moving_between_albums_cleans_orphans() {
    let (_dir, db_path, mut store, _changes_rx) = open_store_with_path();

    let original = library_track("f:\\x\\a.mp3", "A", Some("Solo"), "X", None);
    store
        .apply_scan_batch(std::slice::from_ref(&original))
        .expect("batch applies");
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 7, date_added_nanos = 100
                 WHERE path = 'f:\\x\\a.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    // Move the track to another album/artist via a tag edit.
    let mut moved = original;
    moved.metadata.album_artist = Some("Duo".to_string());
    moved.metadata.album = Some("Y".to_string());
    moved.metadata.year = Some(1970);
    moved.metadata.genre = Some("Prog".to_string());
    store.apply_tag_refresh(&moved).expect("refresh applies");

    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let a = reopened
        .get_track(&moved.id)
        .expect("get_track works")
        .expect("track known after reopen");
    assert_eq!(a.play_count, 7, "history survives the move");
    assert_eq!(
        a.date_added,
        Some(SystemTime::UNIX_EPOCH + Duration::from_nanos(100))
    );

    let (albums, artists): (Vec<(String, String)>, Vec<String>) = reopened
        .with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT album_artist, title FROM albums ORDER BY album_artist")?;
            let albums = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut stmt = conn.prepare("SELECT name FROM artists ORDER BY name")?;
            let artists = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((albums, artists))
        })
        .expect("reading parents must work");
    assert_eq!(
        albums,
        vec![("Duo".to_string(), "Y".to_string())],
        "vacated album X must be gone; target album Y remains"
    );
    assert_eq!(
        artists,
        vec!["Duo".to_string()],
        "artist Solo had no albums left and must be gone"
    );

    let (year, genre): (Option<i64>, Option<String>) = reopened
        .with_connection(|conn| {
            conn.query_row(
                "SELECT year, genre FROM albums WHERE album_artist = 'Duo' AND title = 'Y'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
        })
        .expect("reading the album row must work");
    assert_eq!(year, Some(1970));
    assert_eq!(genre.as_deref(), Some("Prog"));
}

#[test]
fn test_remove_library_path_is_atomic_and_keeps_playlists_dangling() {
    let (_dir, db_path, mut store, _changes_rx) = open_store_with_path();

    // Two roots share artist "Shared"; "m:\one2" guards against prefix traps.
    let batch = vec![
        library_track("m:\\one\\a.mp3", "A", Some("Shared"), "One", None),
        library_track("m:\\one\\b.mp3", "B", Some("Shared"), "One", None),
        library_track("m:\\one2\\c.mp3", "C", Some("Shared"), "Two", None),
        library_track("m:\\other\\d.mp3", "D", Some("Loner"), "Other", None),
    ];
    store.apply_scan_batch(&batch).expect("batch applies");
    for root in ["m:\\one", "m:\\one2"] {
        store
            .with_connection(|conn| {
                conn.execute("INSERT INTO library_paths(path) VALUES (?1)", [root])
            })
            .expect("registering the path must work");
    }

    // A playlist entry pointing into the removed root must survive dangling.
    let removed_id = TrackId("m:\\one\\a.mp3".to_string());
    let playlist_id = store
        .create_playlist("Mix", std::slice::from_ref(&removed_id))
        .expect("playlist creation works");

    let removed = store
        .remove_library_path(std::path::Path::new("m:\\one"))
        .expect("removal works");
    assert_eq!(removed, 2, "exactly that root's two tracks are removed");

    // Unknown roots remove nothing.
    let again = store
        .remove_library_path(std::path::Path::new("m:\\nowhere"))
        .expect("removing an unknown root works");
    assert_eq!(again, 0);

    drop(store);
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();

    let remaining: Vec<String> = reopened
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM tracks ORDER BY path")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("listing tracks must work");
    assert_eq!(
        remaining,
        vec![
            "m:\\one2\\c.mp3".to_string(),
            "m:\\other\\d.mp3".to_string(),
        ],
        "exactly the removed root's tracks are gone; sibling prefixes survive"
    );

    let (albums, artists): (Vec<(String, String)>, Vec<String>) = reopened
        .with_connection(|conn| {
            let mut stmt = conn
                .prepare("SELECT album_artist, title FROM albums ORDER BY album_artist, title")?;
            let albums = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut stmt = conn.prepare("SELECT name FROM artists ORDER BY name")?;
            let artists = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((albums, artists))
        })
        .expect("reading parents must work");
    assert_eq!(
        albums,
        vec![
            ("Loner".to_string(), "Other".to_string()),
            ("Shared".to_string(), "Two".to_string()),
        ],
        "emptied album One is cleaned; albums with surviving tracks stay"
    );
    assert_eq!(
        artists,
        vec!["Loner".to_string(), "Shared".to_string()],
        "orphaned artists go; Shared keeps an album"
    );

    let paths: Vec<String> = reopened
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM library_paths ORDER BY path")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("listing library paths must work");
    assert_eq!(
        paths,
        vec!["m:\\one2".to_string()],
        "the removed root's own record goes; others stay"
    );

    // Playlist entries referencing removed files stay listed so they recover
    // when the files return.
    let playlists = reopened.load_playlists().expect("playlists load");
    let mix = playlists
        .iter()
        .find(|p| p.id == playlist_id)
        .expect("playlist survives");
    assert_eq!(mix.tracks, vec![removed_id], "dangling entry stays listed");
}

/// Record `root` as a library path the way a scan registration does: the raw
/// spelling, because the `library_paths` row is keyed on the text the caller
/// passed and `remove_library_path` deletes that same text.
fn register_library_path(store: &SqliteStore, root: &str) {
    store
        .with_connection(|conn| conn.execute("INSERT INTO library_paths(path) VALUES (?1)", [root]))
        .expect("registering the path must work");
}

/// Every surviving track path, in path order — the whole post-removal truth.
fn remaining_track_paths(store: &SqliteStore) -> Vec<String> {
    store
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM tracks ORDER BY path")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("listing tracks must work")
}

/// A root spelled with a trailing separator names the same folder as the bare
/// spelling, so it must remove the same subtree. The trailing separator is
/// part of the spelling, not part of the membership rule.
#[test]
fn remove_library_path_removes_the_subtree_under_a_trailing_separator_root() {
    let (_dir, mut store, _changes_rx) = open_store();
    store
        .apply_scan_batch(&[
            library_track("m:\\music\\a.mp3", "A", None, "Music", None),
            library_track("m:\\music\\sub\\b.mp3", "B", None, "Music", None),
            library_track("m:\\music2\\c.mp3", "C", None, "Other", None),
            library_track("n:\\elsewhere\\d.mp3", "D", None, "Elsewhere", None),
        ])
        .unwrap();
    register_library_path(&store, "m:\\music\\");

    let removed = store
        .remove_library_path(std::path::Path::new("m:\\music\\"))
        .expect("removal works");

    assert_eq!(
        removed, 2,
        "the trailing-separator root's whole subtree goes, nested files included"
    );
    assert_eq!(
        remaining_track_paths(&store),
        vec!["m:\\music2\\c.mp3", "n:\\elsewhere\\d.mp3"],
        "the byte-prefix sibling and the unrelated root both survive"
    );
    // The read path agrees the folder is now empty, so removal and the folder
    // reads name the same subtree.
    assert!(
        !store
            .folder_has_audio(std::path::Path::new("m:\\music"))
            .unwrap(),
        "the removed folder reports no audio"
    );
    assert!(
        store
            .folder_has_audio(std::path::Path::new("m:\\music2"))
            .unwrap(),
        "the prefix sibling still reports its audio"
    );
    let paths: Vec<String> = store
        .with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM library_paths ORDER BY path")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect()
        })
        .expect("listing library paths must work");
    assert!(paths.is_empty(), "the exact spelling's own record goes");
}

/// A bare root (`m:\`) already ends in its own separator, so every path under
/// that drive belongs to it: the whole drive's subtree goes in one removal.
#[test]
fn remove_library_path_removes_the_subtree_under_a_bare_root() {
    let (_dir, mut store, _changes_rx) = open_store();
    store
        .apply_scan_batch(&[
            library_track("m:\\a.mp3", "A", None, "Root", None),
            library_track("m:\\sub\\b.mp3", "B", None, "Nested", None),
            library_track("m:\\sub\\deep\\c.mp3", "C", None, "Deep", None),
            library_track("n:\\d.mp3", "D", None, "Other", None),
        ])
        .unwrap();

    let removed = store
        .remove_library_path(std::path::Path::new("m:\\"))
        .expect("removal works");

    assert_eq!(
        removed, 3,
        "every path under the bare root goes, at every depth"
    );
    assert_eq!(
        remaining_track_paths(&store),
        vec!["n:\\d.mp3".to_string()],
        "the other drive's track survives"
    );
}

/// A root carrying LIKE metacharacters matches them literally: the one folder
/// whose name contains them is removed, while the spellings an unescaped
/// wildcard would also match survive.
#[test]
fn remove_library_path_matches_like_metacharacters_literally() {
    let (_dir, mut store, _changes_rx) = open_store();
    store
        .apply_scan_batch(&[
            library_track("m:\\chart\\50%_off\\01.mp3", "A", None, "Literal", None),
            // What an unescaped `%` / `_` pair would also match.
            library_track("m:\\chart\\50%_offer\\02.mp3", "B", None, "Decoy", None),
            library_track("m:\\chart\\50%_offx\\03.mp3", "C", None, "Sibling", None),
            library_track("m:\\chart2\\04.mp3", "D", None, "Other", None),
        ])
        .unwrap();
    let root = std::path::Path::new("m:\\chart\\50%_off");
    assert_eq!(
        store.folder_track_count(root).unwrap(),
        1,
        "the read path reports exactly the literal match"
    );

    let removed = store.remove_library_path(root).expect("removal works");

    assert_eq!(removed, 1, "only the literal match is removed");
    assert_eq!(
        remaining_track_paths(&store),
        vec![
            "m:\\chart2\\04.mp3",
            "m:\\chart\\50%_offer\\02.mp3",
            "m:\\chart\\50%_offx\\03.mp3",
        ],
        "wildcard-expanded and byte-prefix spellings all survive"
    );
}

/// A root whose trailing separator differs from the separators in the stored
/// paths must remove exactly the subtree the folder reads report for it —
/// the delete and the reads are one rule.
#[test]
fn remove_library_path_agrees_with_the_folder_reads_for_a_mixed_separator_root() {
    let (_dir, mut store, _changes_rx) = open_store();
    store
        .apply_scan_batch(&[
            library_track("m:\\one\\a.mp3", "A", None, "One", None),
            library_track("m:\\one\\sub\\b.mp3", "B", None, "One", None),
            library_track("m:\\one2\\c.mp3", "C", None, "Two", None),
            library_track("n:\\d.mp3", "D", None, "Other", None),
        ])
        .unwrap();
    let root = std::path::Path::new("m:\\one/");
    let reported = store
        .folder_track_count(root)
        .expect("the read path answers for the mixed separator root");
    assert_eq!(reported, 2, "the read path reports the whole subtree");

    let removed = store.remove_library_path(root).expect("removal works");

    assert_eq!(
        removed, reported,
        "the delete removes exactly what the folder reads report"
    );
    assert_eq!(
        remaining_track_paths(&store),
        vec!["m:\\one2\\c.mp3", "n:\\d.mp3"],
        "the byte-prefix sibling and the unrelated root both survive"
    );
}

/// The sibling-prefix trap, proved on both sides of the removal: the tracks
/// under `m:\music` go, and every track under `m:\music2` — including a nested
/// one — stays, as the folder reads continue to report.
#[test]
fn remove_library_path_never_swallows_a_prefix_sibling_root() {
    let (_dir, mut store, _changes_rx) = open_store();
    store
        .apply_scan_batch(&[
            library_track("m:\\music\\a.mp3", "A", None, "Music", None),
            library_track("m:\\music\\sub\\b.mp3", "B", None, "Music", None),
            library_track("m:\\music2\\c.mp3", "C", None, "Other", None),
            library_track("m:\\music2\\deep\\d.mp3", "D", None, "Deep", None),
        ])
        .unwrap();
    let root = std::path::Path::new("m:\\music");
    let sibling = std::path::Path::new("m:\\music2");
    assert_eq!(store.folder_track_count(root).unwrap(), 2);
    assert_eq!(store.folder_track_count(sibling).unwrap(), 2);

    let removed = store.remove_library_path(root).expect("removal works");

    assert_eq!(removed, 2, "only the root's own tracks go");
    assert!(
        !store.folder_has_audio(root).unwrap(),
        "the removed root reports no audio"
    );
    assert!(
        store.folder_has_audio(sibling).unwrap(),
        "the prefix sibling keeps reporting its audio, nested tracks included"
    );
    assert_eq!(
        store.track_ids_in_folder_tree(sibling).unwrap(),
        vec![
            TrackId("m:\\music2\\c.mp3".to_string()),
            TrackId("m:\\music2\\deep\\d.mp3".to_string()),
        ],
        "the prefix sibling's whole subtree survives"
    );
}

// --- Application Store: artist/album browsing queries (ticket 07) ------------

/// A fixture track with full control over number and year, for browsing
/// orderings.
fn browsing_track(
    path: &str,
    title: &str,
    album_artist: &str,
    album: &str,
    track_number: Option<u32>,
    year: Option<u32>,
) -> Track {
    Track::fresh_scan(
        TrackId::from_path(std::path::Path::new(path)),
        PathBuf::from(path),
        TrackMetadata {
            title: Some(title.to_string()),
            artist: Some(album_artist.to_string()),
            album: Some(album.to_string()),
            album_artist: Some(album_artist.to_string()),
            track_number,
            year,
            ..TrackMetadata::default()
        },
        None,
        None,
        None,
        None,
    )
}

/// An independent Rust-only restatement of the three browsing orderings,
/// computed straight from the fixture list with the former comparators. The
/// SQL queries must agree with this without going through any shared code.
mod reference_order {
    use super::*;
    use std::collections::BTreeSet;

    pub fn artists_az(fixtures: &[Track]) -> Vec<String> {
        let names: BTreeSet<String> = fixtures
            .iter()
            .map(|t| t.metadata.display_album_artist().into_owned())
            .collect();
        names.into_iter().collect()
    }

    pub fn artist_albums(fixtures: &[Track], artist: &str) -> Vec<(String, Option<u32>)> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut albums: Vec<(String, Option<u32>)> = Vec::new();
        for track in fixtures
            .iter()
            .filter(|t| t.metadata.display_album_artist() == artist)
        {
            let title = track.metadata.display_album().into_owned();
            if seen.insert(title.clone()) {
                albums.push((title, track.metadata.year));
            }
        }
        albums.sort_by(|a, b| {
            b.1.unwrap_or(0)
                .cmp(&a.1.unwrap_or(0))
                .then_with(|| a.0.cmp(&b.0))
        });
        albums
    }

    pub fn album_tracks(fixtures: &[Track], artist: &str, title: &str) -> Vec<TrackId> {
        let mut tracks: Vec<&Track> = fixtures
            .iter()
            .filter(|t| {
                t.metadata.display_album_artist() == artist && t.metadata.display_album() == title
            })
            .collect();
        tracks.sort_by(|a, b| {
            a.metadata
                .track_number
                .unwrap_or(0)
                .cmp(&b.metadata.track_number.unwrap_or(0))
                .then_with(|| a.file_path.cmp(&b.file_path))
        });
        tracks.into_iter().map(|t| t.id.clone()).collect()
    }
}

/// The parity fixture library: multiple artists, missing years, same-year
/// title ties, missing track numbers, and duplicate-number ties, listed in
/// the path-ascending insertion order that scans produce.
fn browsing_fixtures() -> Vec<Track> {
    vec![
        browsing_track(
            "f:\\zeta\\new wave\\01 - Opening.mp3",
            "Opening",
            "Zeta",
            "New Wave",
            Some(1),
            Some(2019),
        ),
        browsing_track(
            "f:\\zeta\\new wave\\02 - Middle.mp3",
            "Middle",
            "Zeta",
            "New Wave",
            Some(2),
            Some(2019),
        ),
        // Duplicate track number 2: a tie the path tiebreak must settle.
        browsing_track(
            "f:\\zeta\\new wave\\02b - Encore.mp3",
            "Encore",
            "Zeta",
            "New Wave",
            Some(2),
            Some(2019),
        ),
        // Missing track number sorts before numbered tracks (legacy 0 slot).
        browsing_track(
            "f:\\zeta\\new wave\\zz - Unnumbered.mp3",
            "Unnumbered",
            "Zeta",
            "New Wave",
            None,
            Some(2019),
        ),
        browsing_track(
            "f:\\zeta\\new wave\\10 - Closer.mp3",
            "Closer",
            "Zeta",
            "New Wave",
            Some(10),
            Some(2019),
        ),
        browsing_track(
            "f:\\zeta\\a sides\\1.mp3",
            "A1",
            "Zeta",
            "A Sides",
            Some(1),
            Some(2010),
        ),
        browsing_track(
            "f:\\zeta\\b sides\\1.mp3",
            "B1",
            "Zeta",
            "B Sides",
            Some(1),
            Some(2010),
        ),
        browsing_track(
            "f:\\zeta\\middle era\\1.mp3",
            "M1",
            "Zeta",
            "Middle Era",
            Some(1),
            Some(2005),
        ),
        // Missing year is oldest in the newest-first album order.
        browsing_track(
            "f:\\zeta\\old hits\\1.mp3",
            "O1",
            "Zeta",
            "Old Hits",
            Some(1),
            None,
        ),
        browsing_track(
            "f:\\alpha\\only\\2.mp3",
            "Alpha Two",
            "Alpha",
            "Only",
            Some(2),
            Some(1999),
        ),
        browsing_track(
            "f:\\alpha\\only\\1.mp3",
            "Alpha One",
            "Alpha",
            "Only",
            Some(1),
            Some(1999),
        ),
    ]
}

#[test]
fn test_browsing_queries_match_independent_reference_orderings() {
    // Both sides receive the identical fixtures in the identical insertion
    // order; only the ordering pipelines differ (SQL vs an independent Rust
    // restatement of the former comparators).
    let fixtures = browsing_fixtures();

    // Store side: the whole fixture set through one scan commit.
    let (_dir, mut store, _changes_rx) = open_store();
    store.apply_scan_batch(&fixtures).expect("batch applies");

    // Artists A–Z.
    assert_eq!(
        store
            .all_artists()
            .expect("artists query")
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>(),
        reference_order::artists_az(&fixtures),
        "SQL artist order must match the independent reference ordering"
    );

    // An artist's albums: newest-first (missing year last) then title.
    assert_eq!(
        store
            .artist_albums("Zeta")
            .expect("albums query")
            .iter()
            .map(|a| (a.title.clone(), a.year))
            .collect::<Vec<_>>(),
        reference_order::artist_albums(&fixtures, "Zeta"),
        "SQL album order must match the independent reference ordering"
    );

    // Album tracks: number then filename, missing numbers first, ties by path.
    assert_eq!(
        store
            .album_tracks("Zeta", "New Wave")
            .expect("album tracks query")
            .iter()
            .map(|t| t.id.clone())
            .collect::<Vec<_>>(),
        reference_order::album_tracks(&fixtures, "Zeta", "New Wave"),
        "SQL album-track order must match the independent reference ordering"
    );
}

#[test]
fn test_all_artists_lists_names_az_with_canonical_album_keys() {
    let (_dir, mut store, _changes_rx) = open_store();

    // Fresh store: no artists.
    assert!(
        store.all_artists().expect("empty query").is_empty(),
        "a fresh store lists no artists"
    );

    // Insertion deliberately out of alphabetical order, with years chosen so
    // canonical album order differs from first-added order.
    store
        .apply_scan_batch(&[
            browsing_track("f:\\z\\1.mp3", "Z1", "Zulu", "Late", Some(1), Some(2020)),
            browsing_track("f:\\a\\2.mp3", "A2", "Alpha", "Early", Some(2), Some(1980)),
            browsing_track("f:\\a\\1.mp3", "A1", "Alpha", "Dated", Some(1), None),
            browsing_track("f:\\a\\3.mp3", "A3", "Alpha", "Early", Some(3), Some(1980)),
        ])
        .expect("batch applies");

    let artists = store.all_artists().expect("artists query");
    assert_eq!(
        artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["Alpha", "Zulu"],
        "artists list name-ascending"
    );

    let alpha = &artists[0];
    assert_eq!(
        alpha.albums,
        vec!["Alpha - Early".to_string(), "Alpha - Dated".to_string(),],
        "an artist's embedded keys follow the canonical album order \
         (newest-first, missing year last, then title)"
    );
}

// --- Paged browse reads (paginate-browse-columns 02-05) -------------------
//
// Bounded windows + authoritative totals pinned against real SQLite: window
// slicing, exact descending order per direction, and count semantics.

#[test]
fn test_artists_window_slices_name_order_both_directions_and_counts() {
    let (_dir, mut store, _changes_rx) = open_store();

    // Fresh store: no artists.
    assert_eq!(artists_count(&store), 0);
    assert!(
        artists_window(&store, SortDirection::Ascending, 0, 10).is_empty(),
        "a fresh store has no artist rows"
    );

    // Insertion deliberately out of alphabetical order, with years chosen so
    // canonical album order differs from first-added order.
    store
        .apply_scan_batch(&[
            browsing_track("f:\\z\\1.mp3", "Z1", "Zulu", "Late", Some(1), Some(2020)),
            browsing_track("f:\\a\\2.mp3", "A2", "Alpha", "Early", Some(2), Some(1980)),
            browsing_track("f:\\a\\1.mp3", "A1", "Alpha", "Dated", Some(1), None),
            browsing_track("f:\\a\\3.mp3", "A3", "Alpha", "Early", Some(3), Some(1980)),
            browsing_track("f:\\b\\1.mp3", "B1", "Bravo", "Debut", Some(1), Some(1995)),
        ])
        .expect("batch applies");

    assert_eq!(artists_count(&store), 3);

    let asc_first = artists_window(&store, SortDirection::Ascending, 0, 2);
    assert_eq!(
        asc_first
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Bravo"],
        "the window is name-ascending byte-wise"
    );
    // The windowed artist rows keep their canonical album keys, exactly like
    // `all_artists` (newest-first, missing year last, then title).
    assert_eq!(
        asc_first[0].albums,
        vec!["Alpha - Early".to_string(), "Alpha - Dated".to_string()],
        "an artist's embedded keys arrive canonical in a window too"
    );

    let asc_tail = artists_window(&store, SortDirection::Ascending, 2, 10);
    assert_eq!(
        asc_tail.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["Zulu"],
        "the tail window returns the remaining rows"
    );

    let desc_first = artists_window(&store, SortDirection::Descending, 0, 2);
    assert_eq!(
        desc_first
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Zulu", "Bravo"],
        "descending is exact descending SQL order, not an in-memory reversal"
    );

    let desc_tail = artists_window(&store, SortDirection::Descending, 2, 10);
    assert_eq!(
        desc_tail
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha"],
        "the descending tail completes the reversed list"
    );

    // Windows past the end yield empty slices in both directions.
    assert!(
        artists_window(&store, SortDirection::Ascending, 100, 5).is_empty(),
        "offset past the end yields an empty window"
    );
    assert!(
        artists_window(&store, SortDirection::Descending, 100, 5).is_empty(),
        "descending past the end yields an empty window"
    );
}

#[test]
fn test_albums_window_flat_order_slices_and_counts_in_both_directions() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            browsing_track("f:\\z\\1.mp3", "Z1", "Zulu", "Late", Some(1), Some(2020)),
            browsing_track("f:\\a\\1.mp3", "A1", "Alpha", "Old", Some(1), Some(1980)),
            browsing_track("f:\\a\\2.mp3", "A2", "Alpha", "New", Some(2), Some(2021)),
            browsing_track("f:\\b\\1.mp3", "B1", "Bravo", "Debut", Some(1), None),
        ])
        .expect("batch applies");

    assert_eq!(albums_count(&store), 4);

    // Canonical flat browsing order: album artist ascending, year descending
    // with missing years last, then title ascending.
    let asc = albums_window(&store, SortDirection::Ascending, 0, 10);
    assert_eq!(
        asc.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["New", "Old", "Debut", "Late"],
        "flat browsing order: artist asc, year desc, missing year last, title asc"
    );

    // Each album carries its full track ids in album-track order.
    let new = asc.iter().find(|a| a.title == "New").unwrap();
    assert_eq!(new.tracks, vec![TrackId("f:\\a\\2.mp3".to_string())]);
    assert_eq!(new.artist, "Alpha");

    let desc = albums_window(&store, SortDirection::Descending, 0, 10);
    assert_eq!(
        desc.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["Late", "Debut", "Old", "New"],
        "descending is the exact reversal of the flat browsing order"
    );

    // Slicing a window mid-list keeps the order alignment.
    let asc_mid = albums_window(&store, SortDirection::Ascending, 1, 2);
    assert_eq!(
        asc_mid.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["Old", "Debut"],
        "offset/limit slicing lands on the canonical order"
    );
}

#[test]
fn test_genres_window_counts_and_orders_both_directions() {
    let (_dir, mut store, _changes_rx) = open_store();

    // A track tagged "Rock; Jazz" counts once per entry; byte-wise
    // case-sensitivity means "rock" is a distinct entry from "Rock".
    store
        .apply_scan_batch(&[
            genre_track("f:\\1.mp3", "One", "Duo", "A", Some("Rock; Jazz")),
            genre_track("f:\\2.mp3", "Two", "Duo", "A", Some("Ambient")),
            genre_track("f:\\3.mp3", "Three", "Duo", "A", Some("rock")),
        ])
        .expect("batch applies");

    assert_eq!(genres_count(&store), 4);

    let asc = genres_window(&store, SortDirection::Ascending, 0, 10);
    assert_eq!(
        asc.iter().map(|g| g.genre.as_str()).collect::<Vec<_>>(),
        ["Ambient", "Jazz", "Rock", "rock"],
        "name-ascending byte-wise, semicolon segments split per entry"
    );
    assert_eq!(
        asc[2].tracks, 1,
        "the case-sensitive Rock entry holds one track"
    );

    let desc = genres_window(&store, SortDirection::Descending, 0, 10);
    assert_eq!(
        desc.iter().map(|g| g.genre.as_str()).collect::<Vec<_>>(),
        ["rock", "Rock", "Jazz", "Ambient"],
        "descending is the exact reversed genre order"
    );
}

#[test]
fn test_artists_in_genre_window_and_count_are_scoped_and_ordered() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            genre_track("f:\\r1.mp3", "R1", "Rockers", "RB", Some("Rock")),
            genre_track("f:\\r2.mp3", "R2", "Rockers", "RB2", Some("Rock")),
            genre_track("f:\\a1.mp3", "A1", "Ambientia", "AB", Some("Ambient")),
            genre_track("f:\\m1.mp3", "M1", "Rockers", "RB", Some("Metal")),
            // A Pop artist with no Rock track: Pops never appears for Rock.
            genre_track("f:\\p1.mp3", "P1", "Pops", "PB", Some("Pop")),
        ])
        .expect("batch applies");

    assert_eq!(
        artists_in_genre_count(&store, "Rock"),
        1,
        "the count is distinct artists — Rockers is one artist with two Rock albums"
    );
    assert_eq!(artists_in_genre_count(&store, "Ambient"), 1);
    assert_eq!(
        artists_in_genre_count(&store, "Nothing"),
        0,
        "unknown genres count zero"
    );

    let asc = artists_in_genre_window(&store, "Rock", SortDirection::Ascending, 0, 10);
    assert_eq!(
        asc.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["Rockers"],
        "only artists with a Rock album appear, name-ascending"
    );
    assert_eq!(
        asc[0].albums,
        vec!["Rockers - RB".to_string(), "Rockers - RB2".to_string()],
        "the artist carries its genre-scoped album keys in canonical order"
    );

    let desc = artists_in_genre_window(&store, "Rock", SortDirection::Descending, 0, 10);
    assert_eq!(
        desc.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["Rockers"],
        "descending keeps the same scoped set in reversed order"
    );
}

#[test]
fn test_artist_albums_in_genre_window_and_count_are_scoped_and_ordered() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            genre_track("f:\\r1.mp3", "R1", "Rockers", "Old", Some("Rock")),
            genre_track("f:\\r2.mp3", "R2", "Rockers", "New", Some("Rock")),
            genre_track("f:\\m1.mp3", "M1", "Rockers", "Old", Some("Metal")),
            genre_track("f:\\m2.mp3", "M2", "Rockers", "Metal Album", Some("Metal")),
        ])
        .expect("batch applies");

    assert_eq!(
        artist_albums_in_genre_count(&store, "Rockers", "Rock"),
        2,
        "only albums holding a Rock track count"
    );
    assert_eq!(
        artist_albums_in_genre_count(&store, "Rockers", "Nothing"),
        0
    );

    let asc =
        artist_albums_in_genre_window(&store, "Rockers", "Rock", SortDirection::Ascending, 0, 10);
    assert_eq!(
        asc.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["New", "Old"],
        "canonical browsing order: year descending with missing years last, title asc"
    );
    assert_eq!(
        asc[1].tracks,
        vec![TrackId("f:\\r1.mp3".to_string())],
        "each album carries only its matching track ids"
    );

    let desc =
        artist_albums_in_genre_window(&store, "Rockers", "Rock", SortDirection::Descending, 0, 10);
    assert_eq!(
        desc.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["Old", "New"],
        "descending is the exact reversed canonical order"
    );
}

#[test]
fn test_artist_albums_in_genre_window_slices_offset() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            genre_track("f:\\1.mp3", "1", "A", "Zeta", Some("Rock")),
            genre_track("f:\\2.mp3", "2", "A", "Alpha", Some("Rock")),
            genre_track("f:\\3.mp3", "3", "A", "Mid", Some("Rock")),
        ])
        .expect("batch applies");

    let mid = artist_albums_in_genre_window(&store, "A", "Rock", SortDirection::Ascending, 1, 1);
    assert_eq!(
        mid.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["Mid"],
        "offset/limit slicing lands on the canonical order"
    );
}

#[test]
fn test_artist_albums_in_genre_unknown_scopes_yield_empty() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[genre_track("f:\\1.mp3", "1", "A", "Zeta", Some("Rock"))])
        .expect("batch applies");

    assert!(
        artist_albums_in_genre_window(&store, "Nobody", "Rock", SortDirection::Ascending, 0, 10)
            .is_empty(),
        "unknown artists yield no albums"
    );
    assert!(
        artist_albums_in_genre_window(&store, "A", "Nothing", SortDirection::Ascending, 0, 10)
            .is_empty(),
        "unknown genres yield no albums"
    );
}

#[test]
fn test_artist_albums_orders_newest_first_missing_year_last_title_tiebreak() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            browsing_track("f:\\n\\1.mp3", "N1", "Nova", "Undated", Some(1), None),
            browsing_track("f:\\n\\2.mp3", "N2", "Nova", "Twins B", Some(1), Some(2000)),
            browsing_track("f:\\n\\3.mp3", "N3", "Nova", "Old", Some(1), Some(1990)),
            browsing_track("f:\\n\\4.mp3", "N4", "Nova", "Twins A", Some(1), Some(2000)),
            browsing_track("f:\\n\\5.mp3", "N5", "Nova", "Latest", Some(1), Some(2021)),
        ])
        .expect("batch applies");

    let albums = store.artist_albums("Nova").expect("albums query");
    assert_eq!(
        albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["Latest", "Twins A", "Twins B", "Old", "Undated"],
        "year descending with missing year last, then title ascending"
    );

    // Every album carries its own tracks in album-track order.
    let twins_a = albums.iter().find(|a| a.title == "Twins A").unwrap();
    assert_eq!(twins_a.tracks.len(), 1, "membership stays per-album");

    // Unknown artists browse to an empty list, not an error.
    assert!(
        store
            .artist_albums("Nobody")
            .expect("unknown artist")
            .is_empty(),
        "unknown artists yield no albums"
    );
}

#[test]
fn test_album_tracks_orders_missing_numbers_first_then_path() {
    let (_dir, mut store, _changes_rx) = open_store();

    // Insertion order deliberately scrambled relative to both number and
    // path; the query must impose the canonical order regardless.
    store
        .apply_scan_batch(&[
            browsing_track("f:\\al\\09_zeta.mp3", "Nine", "Duo", "Y", Some(9), None),
            browsing_track("f:\\al\\zz_unnumbered.mp3", "Ghost", "Duo", "Y", None, None),
            browsing_track("f:\\al\\05_beta.mp3", "Five B", "Duo", "Y", Some(5), None),
            browsing_track("f:\\al\\01_alpha.mp3", "One", "Duo", "Y", Some(1), None),
            browsing_track("f:\\al\\05_alpha.mp3", "Five A", "Duo", "Y", Some(5), None),
        ])
        .expect("batch applies");

    let tracks = store.album_tracks("Duo", "Y").expect("album tracks query");
    assert_eq!(
        tracks.iter().map(|t| t.id.0.as_str()).collect::<Vec<_>>(),
        [
            "f:\\al\\zz_unnumbered.mp3", // missing number: legacy 0 slot, first
            "f:\\al\\01_alpha.mp3",
            "f:\\al\\05_alpha.mp3", // duplicate numbers tie-break by path
            "f:\\al\\05_beta.mp3",
            "f:\\al\\09_zeta.mp3",
        ],
        "track number ascending with missing numbers first, path tiebreak"
    );

    // Unknown albums browse to an empty list, not an error.
    assert!(
        store
            .album_tracks("Duo", "Nonexistent")
            .expect("unknown album")
            .is_empty(),
        "unknown albums yield no tracks"
    );
}

#[test]
fn test_browsing_works_straight_from_a_freshly_reopened_store() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        store
            .apply_scan_batch(&[
                browsing_track("f:\\x\\2.mp3", "Two", "Ithaca", "LP", Some(2), Some(1977)),
                browsing_track("f:\\x\\1.mp3", "One", "Ithaca", "LP", Some(1), Some(1977)),
            ])
            .expect("batch applies");
    } // store dropped: connection closed like an app restart.

    // Reopened cold — no hydration, no warm-up of any kind.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
    let artists = reopened.all_artists().expect("artists query");
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].name, "Ithaca");
    assert_eq!(artists[0].albums, vec!["Ithaca - LP".to_string()]);

    let albums = reopened.artist_albums("Ithaca").expect("albums query");
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].title, "LP");
    assert_eq!(albums[0].year, Some(1977));
    assert_eq!(
        albums[0].tracks,
        vec![
            TrackId("f:\\x\\1.mp3".to_string()),
            TrackId("f:\\x\\2.mp3".to_string()),
        ],
        "album membership hydrates in browsing order without warm-up"
    );

    let tracks = reopened.album_tracks("Ithaca", "LP").expect("tracks query");
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].metadata.title.as_deref(), Some("One"));
    assert_eq!(tracks[1].metadata.title.as_deref(), Some("Two"));
}

// --- Application Store: genre read-model queries (design-handoff issue 02) --

/// A browsing fixture with explicit per-track genre control (the genre read
/// model derives from per-track genre metadata, not the album's derived
/// genre column).
fn genre_track(
    path: &str,
    title: &str,
    album_artist: &str,
    album: &str,
    genre: Option<&str>,
) -> Track {
    let base = browsing_track(path, title, album_artist, album, None, None);
    Track::fresh_scan(
        base.id,
        base.file_path,
        TrackMetadata {
            genre: genre.map(str::to_string),
            ..base.metadata
        },
        base.duration,
        base.sample_rate,
        base.channels,
        base.date_added,
    )
}

#[test]
fn test_genre_counts_aggregate_per_track_genres_and_skip_missing() {
    let (_dir, mut store, _changes_rx) = open_store();

    // A fresh store answers with no genre rows at all — no phantom entries.
    assert!(
        store
            .genre_counts()
            .expect("fresh store genre counts")
            .is_empty(),
        "a fresh store has no genres"
    );

    store
        .apply_scan_batch(&[
            genre_track("f:\\g\\1.mp3", "One", "Alpha", "LP", Some("Rock")),
            genre_track("f:\\g\\2.mp3", "Two", "Alpha", "LP", Some("Rock")),
            genre_track("f:\\g\\3.mp3", "Three", "Beta", "EP", Some("Jazz")),
            // Tracks without a genre (None or empty) aggregate into nothing.
            genre_track("f:\\g\\4.mp3", "Four", "Gamma", "Single", None),
            genre_track("f:\\g\\5.mp3", "Five", "Gamma", "Single", Some("")),
        ])
        .expect("batch applies");

    let counts = store.genre_counts().expect("genre counts query");
    assert_eq!(
        counts
            .iter()
            .map(|g| (g.genre.as_str(), g.tracks))
            .collect::<Vec<_>>(),
        [("Jazz", 1), ("Rock", 2)],
        "genres name-ascending with per-track counts; missing/empty genres excluded"
    );

    // The sidebar's total-genres count is the number of aggregated rows.
    assert_eq!(counts.len(), 2, "total genre count for the sidebar");
}

/// A semicolon-separated genre tag splits into independent entries: the
/// sidebar aggregates each entry, browsing by any entry finds the track, and
/// a longer name containing the same letters (`"Indie Rock"`) never matches
/// `"Rock"`.
#[test]
fn test_genre_semicolon_split_yields_independent_entries() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            // Multi-value tags: whitespace around `;` trims and empty
            // segments drop, while the raw column round-trips untouched.
            genre_track("f:\\g\\1.mp3", "One", "Alpha", "LP", Some("Rock; Jazz")),
            genre_track("f:\\g\\2.mp3", "Two", "Alpha", "LP", Some("Rock;;Punk")),
            genre_track("f:\\g\\3.mp3", "Three", "Beta", "EP", Some("Indie Rock")),
            genre_track("f:\\g\\4.mp3", "Four", "Gamma", "Single", Some("Jazz")),
        ])
        .expect("batch applies");

    let counts = store.genre_counts().expect("genre counts query");
    assert_eq!(
        counts
            .iter()
            .map(|g| (g.genre.as_str(), g.tracks))
            .collect::<Vec<_>>(),
        [("Indie Rock", 1), ("Jazz", 2), ("Punk", 1), ("Rock", 2)],
        "each trimmed `;`-separated entry is its own genre row, counted once per track"
    );

    // Browsing by any single entry finds the multi-tag track.
    let rock_artists = store.artists_in_genre("Rock").expect("genre artists query");
    assert_eq!(
        rock_artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha"],
        "the Rock;Jazz and Rock;;Punk tracks match Rock browsing; Indie Rock does not"
    );
    let jazz_artists = store.artists_in_genre("Jazz").expect("genre artists query");
    assert_eq!(
        jazz_artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Gamma"],
        "the same Rock;Jazz track also matches Jazz browsing"
    );

    let albums = store
        .artist_albums_in_genre("Alpha", "Jazz")
        .expect("genre albums query");
    assert_eq!(
        albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
        ["LP"],
        "only albums holding a Jazz-entry track list under Jazz"
    );
    assert_eq!(
        albums[0]
            .tracks
            .iter()
            .map(|t| t.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\g\\1.mp3"],
        "only the track carrying the Jazz entry is a member"
    );

    // A genre entry that merely contains the name does not match.
    assert!(
        store
            .artist_albums_in_genre("Beta", "Rock")
            .expect("no overmatch")
            .is_empty(),
        "`Indie Rock` is its own entry, not a match for `Rock`"
    );

    // The sidebar total counts entries, not raw tags.
    let counts = store.library_counts().expect("library counts query");
    assert_eq!(
        counts.genres, 4,
        "distinct split entries across the library"
    );
}

#[test]
fn test_artists_in_genre_returns_only_matching_artists_and_album_keys() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            // Alpha: one Rock album plus a Jazz album — only the Rock key
            // survives the filter.
            genre_track("f:\\g\\1.mp3", "One", "Alpha", "Rock LP", Some("Rock")),
            genre_track("f:\\g\\2.mp3", "Two", "Alpha", "Rock LP", Some("Rock")),
            genre_track("f:\\g\\3.mp3", "Three", "Alpha", "Jazz EP", Some("Jazz")),
            // Beta has Rock too and sorts after Alpha.
            genre_track("f:\\g\\4.mp3", "Four", "Beta", "B LP", Some("Rock")),
            // Gamma is Jazz-only: absent from the Rock listing.
            genre_track("f:\\g\\5.mp3", "Five", "Gamma", "G EP", Some("Jazz")),
        ])
        .expect("batch applies");

    let artists = store.artists_in_genre("Rock").expect("genre artists query");
    assert_eq!(
        artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
        ["Alpha", "Beta"],
        "only artists with a Rock track, name-ascending"
    );
    assert_eq!(
        artists[0].albums,
        vec!["Alpha - Rock LP".to_string()],
        "an artist's album keys carry only albums holding a matching track"
    );
    assert_eq!(artists[1].albums, vec!["Beta - B LP".to_string()]);

    // An unknown genre lists nothing rather than erroring.
    assert!(
        store
            .artists_in_genre("Techno")
            .expect("unknown genre")
            .is_empty(),
        "unknown genres yield no artists"
    );
}

#[test]
fn test_artist_albums_in_genre_returns_matching_albums_with_matching_track_ids() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            // Rock LP: two Rock tracks and one Jazz track — the album lists
            // with only its two Rock track ids.
            genre_track("f:\\g\\2.mp3", "Two", "Alpha", "Rock LP", Some("Rock")),
            genre_track("f:\\g\\1.mp3", "One", "Alpha", "Rock LP", Some("Rock")),
            genre_track("f:\\g\\3.mp3", "Three", "Alpha", "Rock LP", Some("Jazz")),
            // Jazz EP: no Rock tracks — absent from the Rock listing.
            genre_track("f:\\g\\4.mp3", "Four", "Alpha", "Jazz EP", Some("Jazz")),
        ])
        .expect("batch applies");

    let albums = store
        .artist_albums_in_genre("Alpha", "Rock")
        .expect("genre albums query");
    assert_eq!(albums.len(), 1, "only albums holding a matching track");
    assert_eq!(albums[0].title, "Rock LP");
    assert_eq!(
        albums[0].tracks,
        vec![
            TrackId("f:\\g\\1.mp3".to_string()),
            TrackId("f:\\g\\2.mp3".to_string()),
        ],
        "matching track ids in album-track order, non-matching ids excluded"
    );

    // Unknown artists or genres browse to an empty list, not an error.
    assert!(
        store
            .artist_albums_in_genre("Beta", "Rock")
            .expect("unknown artist")
            .is_empty()
    );
    assert!(
        store
            .artist_albums_in_genre("Alpha", "Techno")
            .expect("unknown genre")
            .is_empty()
    );
}

#[test]
fn test_album_tracks_in_genre_returns_matching_tracks_in_canonical_order() {
    let (_dir, _, mut store, _changes_rx) = open_store_with_path();

    // Insertion order deliberately scrambled relative to track number; the
    // genre-filtered query must impose the canonical album-track order.
    store
        .apply_scan_batch(&[
            genre_track("f:\\g\\09.mp3", "Nine", "Alpha", "LP", Some("Rock")),
            genre_track("f:\\g\\05_beta.mp3", "Five B", "Alpha", "LP", Some("Rock")),
            genre_track("f:\\g\\01.mp3", "One", "Alpha", "LP", Some("Jazz")),
        ])
        .expect("batch applies");

    let tracks = store
        .album_tracks_in_genre("Alpha", "LP", "Rock")
        .expect("genre album tracks query");
    assert_eq!(
        tracks.iter().map(|t| t.id.0.as_str()).collect::<Vec<_>>(),
        ["f:\\g\\05_beta.mp3", "f:\\g\\09.mp3"],
        "matching tracks in canonical order; the Jazz track is excluded"
    );

    // Unknown albums or genres yield no rows.
    assert!(
        store
            .album_tracks_in_genre("Alpha", "Nonexistent", "Rock")
            .expect("unknown album")
            .is_empty()
    );
    assert!(
        store
            .album_tracks_in_genre("Alpha", "LP", "Techno")
            .expect("unknown genre")
            .is_empty()
    );
}

// --- Application Store: entity hit queries (search-across-library-entity-columns issue 01) --

/// The entity-hit fixture library: name-hit albums (by artist and by title),
/// track-hit albums (neither name matches), a descendant-only album that must
/// stay out, non-Latin names, and literal wildcard characters.
fn entity_hit_fixtures() -> Vec<Track> {
    vec![
        browsing_track(
            "f:\\z\\nw\\01 - Opening.mp3",
            "Opening",
            "Zeta",
            "New Wave",
            Some(1),
            Some(2019),
        ),
        browsing_track(
            "f:\\z\\nw\\02 - Middle.mp3",
            "Middle",
            "Zeta",
            "New Wave",
            Some(2),
            Some(2019),
        ),
        // Missing number sorts before numbered tracks (legacy 0 slot).
        browsing_track(
            "f:\\z\\nw\\zz - Unnumbered.mp3",
            "Unnumbered",
            "Zeta",
            "New Wave",
            None,
            Some(2019),
        ),
        browsing_track(
            "f:\\z\\a sides\\1.mp3",
            "A1",
            "Zeta",
            "A Sides",
            Some(1),
            Some(2010),
        ),
        browsing_track(
            "f:\\z\\old hits\\1.mp3",
            "O1",
            "Zeta",
            "Old Hits",
            Some(1),
            None,
        ),
        browsing_track(
            "f:\\a\\only\\1.mp3",
            "Alpha One",
            "Alpha",
            "Only",
            Some(1),
            Some(1999),
        ),
        browsing_track(
            "f:\\a\\only\\2.mp3",
            "Alpha Two",
            "Alpha",
            "Only",
            Some(2),
            Some(1999),
        ),
        browsing_track(
            "f:\\v\\songs\\1.mp3",
            "Summer",
            "Various Artists",
            "Songs",
            Some(1),
            Some(2020),
        ),
        browsing_track(
            "f:\\t\\ballet\\1.mp3",
            "Лебединое озеро",
            "Чайковский",
            "Балеты",
            Some(1),
            Some(1890),
        ),
        browsing_track(
            "f:\\p\\stage\\1.mp3",
            "100% Live",
            "Idol",
            "Stage",
            Some(1),
            Some(2001),
        ),
    ]
}

/// Seed the entity-hit fixture library and return the store.
fn seeded_entity_hit_store() -> (tempfile::TempDir, riff_infra::store::SqliteStore) {
    let (dir, _, mut store, _changes_rx) = open_store_with_path();
    store
        .apply_scan_batch(&entity_hit_fixtures())
        .expect("fixtures apply");
    (dir, store)
}

/// The `(artist, title)` key tuples of a hit-album listing, for assertion.
fn hit_album_keys(albums: &[Album]) -> Vec<(&str, &str)> {
    albums
        .iter()
        .map(|album| (album.artist.as_str(), album.title.as_str()))
        .collect()
}

#[test]
fn test_hit_albums_list_name_hits_and_track_hits_in_canonical_order() {
    let (_dir, store) = seeded_entity_hit_store();

    // A name-hit album (its title matches) lists with ALL of its hit tracks.
    let wave = hit_albums(&store, "wave", 0, 100);
    assert_eq!(
        hit_album_keys(&wave),
        [("Zeta", "New Wave")],
        "a title name-hit appears, in canonical order"
    );
    assert_eq!(
        wave[0]
            .tracks
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        [
            "f:\\z\\nw\\zz - Unnumbered.mp3",
            "f:\\z\\nw\\01 - Opening.mp3",
            "f:\\z\\nw\\02 - Middle.mp3",
        ],
        "the album carries its hit tracks in canonical album-track order \
         (missing numbers first, then number, then path)"
    );
    assert_eq!(
        hit_albums_count(&store, "wave"),
        1,
        "count agrees with the listing"
    );

    // An artist name-hit surfaces every album of that artist.
    let zeta = hit_albums(&store, "z", 0, 100);
    assert_eq!(
        hit_album_keys(&zeta),
        [
            ("Zeta", "New Wave"),
            ("Zeta", "A Sides"),
            ("Zeta", "Old Hits")
        ],
        "album artist name-hits list every album in canonical browsing order \
         (year descending, missing year last, then title)"
    );

    // A track-hit album — neither its artist nor its title matches — still
    // appears, carrying only the matching track.
    let alpha_one = hit_albums(&store, "alpha one", 0, 100);
    assert_eq!(
        hit_album_keys(&alpha_one),
        [("Alpha", "Only")],
        "a track hit surfaces its album (upward propagation)"
    );
    assert_eq!(
        alpha_one[0]
            .tracks
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\a\\only\\1.mp3"],
        "only the matching track rides along"
    );
    assert!(
        hit_albums(&store, "zzz-no-match", 0, 100).is_empty(),
        "an album whose track is the only match must not appear for other queries"
    );
}

#[test]
fn test_hit_albums_matching_is_literal_and_case_insensitive_including_non_latin() {
    let (_dir, store) = seeded_entity_hit_store();

    // Non-Latin name matching folds case like the tracks' search text.
    for query in ["чайковский", "ЧАЙКОВСКИЙ"] {
        assert_eq!(
            hit_album_keys(&hit_albums(&store, query, 0, 100)),
            [("Чайковский", "Балеты")],
            "non-Latin album artist matching is case-insensitive for {query:?}"
        );
    }

    // Non-Latin track matching surfaces the album upward.
    assert_eq!(
        hit_album_keys(&hit_albums(&store, "лебединое", 0, 100)),
        [("Чайковский", "Балеты")],
        "a non-Latin track hit surfaces its album"
    );

    // Literal `%` and `_` have no wildcard meaning: `%` matches only the
    // track that actually contains the character — it must NOT match every
    // album the way LIKE would.
    assert_eq!(
        hit_album_keys(&hit_albums(&store, "100%", 0, 100)),
        [("Idol", "Stage")],
        "a literal percent matches the track carrying it"
    );
    assert_eq!(
        hit_album_keys(&hit_albums(&store, "%", 0, 100)),
        [("Idol", "Stage")],
        "bare % matches exactly the row carrying the character (no LIKE semantics)"
    );
    assert!(
        hit_albums(&store, "_", 0, 100).is_empty(),
        "bare _ must not match every row (no LIKE semantics)"
    );
}

#[test]
fn test_hit_albums_windows_are_bounded_with_correct_totals() {
    let (_dir, store) = seeded_entity_hit_store();

    // "z" hits three Zeta albums; windows slice that set deterministically.
    assert_eq!(
        hit_albums_count(&store, "z"),
        3,
        "the count is authoritative for the windowing"
    );
    assert_eq!(
        hit_album_keys(&hit_albums(&store, "z", 0, 2)),
        [("Zeta", "New Wave"), ("Zeta", "A Sides")],
        "the first window takes the first two canonical rows"
    );
    assert_eq!(
        hit_album_keys(&hit_albums(&store, "z", 2, 10)),
        [("Zeta", "Old Hits")],
        "the second window takes the remainder"
    );
    assert_eq!(
        hit_album_keys(&hit_albums(&store, "z", 2, 1)),
        [("Zeta", "Old Hits")],
        "an offset mid-library serves exactly the requested row"
    );
    assert!(
        hit_albums(&store, "z", 99, 10).is_empty(),
        "a window past the end serves nothing"
    );

    // Unknown-query windows degrade to empty, never an error.
    assert!(hit_albums(&store, "no such album", 0, 100).is_empty());
}

/// The `(name, [album keys])` shape of a hit-artist listing, for assertion.
fn hit_artist_keys(artists: &[Artist]) -> Vec<(String, Vec<String>)> {
    artists
        .iter()
        .map(|artist| (artist.name.clone(), artist.albums.clone()))
        .collect()
}

#[test]
fn test_hit_artists_list_name_hits_and_album_hits_with_only_hit_album_keys() {
    let (_dir, store) = seeded_entity_hit_store();

    // A name-hit artist carries every album (all are name-hits too), in
    // canonical per-artist order.
    assert_eq!(
        hit_artist_keys(&hit_artists(&store, "zeta", 0, 100)),
        [(
            "Zeta".to_string(),
            vec![
                "Zeta - New Wave".to_string(),
                "Zeta - A Sides".to_string(),
                "Zeta - Old Hits".to_string(),
            ],
        )],
        "a name-hit artist expands into all of its albums"
    );

    // An artist reached only through a track hit appears with only the
    // hit-album key.
    assert_eq!(
        hit_artist_keys(&hit_artists(&store, "alpha one", 0, 100)),
        [("Alpha".to_string(), vec!["Alpha - Only".to_string()])],
        "a track hit surfaces its artist upward with only its hit-album key"
    );

    // Multiple artists in name-ascending order; Zeta's two hit albums ride
    // in canonical order (year descending, missing year last).
    assert_eq!(
        hit_artist_keys(&hit_artists(&store, "1", 0, 100)),
        [
            ("Idol".to_string(), vec!["Idol - Stage".to_string()]),
            (
                "Zeta".to_string(),
                vec!["Zeta - A Sides".to_string(), "Zeta - Old Hits".to_string(),],
            ),
        ],
        "hit artists list name-ascending, each carrying only its hit-album keys"
    );

    // Non-Latin: a track hit surfaces its artist.
    assert_eq!(
        hit_artist_keys(&hit_artists(&store, "лебединое", 0, 100)),
        [(
            "Чайковский".to_string(),
            vec!["Чайковский - Балеты".to_string()]
        )],
        "a non-Latin track hit surfaces its artist"
    );

    // No match: nobody, not an error.
    assert!(hit_artists(&store, "zzz-no-match", 0, 100).is_empty());
    assert_eq!(
        hit_artists_count(&store, "zeta").to_string(),
        "1",
        "count agrees with the listing"
    );
    assert_eq!(
        hit_artists_count(&store, "1"),
        2,
        "count agrees with the multi-artist listing"
    );
}

#[test]
fn test_hit_artists_windows_are_bounded_with_correct_totals() {
    let (_dir, store) = seeded_entity_hit_store();

    assert_eq!(
        hit_artists_count(&store, "1"),
        2,
        "the count is authoritative for the windowing"
    );
    assert_eq!(
        hit_artist_keys(&hit_artists(&store, "1", 0, 1)),
        [("Idol".to_string(), vec!["Idol - Stage".to_string()])],
        "the first window takes the first canonical artist"
    );
    assert_eq!(
        hit_artist_keys(&hit_artists(&store, "1", 1, 1)),
        [(
            "Zeta".to_string(),
            vec!["Zeta - A Sides".to_string(), "Zeta - Old Hits".to_string(),],
        )],
        "the second window takes the next artist with its hit-album keys intact"
    );
    assert!(
        hit_artists(&store, "1", 9, 10).is_empty(),
        "a window past the end serves nothing"
    );
}

#[test]
fn test_album_hit_tracks_list_only_the_matching_tracks_in_canonical_order() {
    let (_dir, store) = seeded_entity_hit_store();

    // A name-hit album's tracks all match (the album title lives in each
    // track's search text), in canonical album-track order.
    assert_eq!(
        store
            .album_hit_tracks("Zeta", "New Wave", "wave")
            .expect("album hit tracks")
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        [
            "f:\\z\\nw\\zz - Unnumbered.mp3",
            "f:\\z\\nw\\01 - Opening.mp3",
            "f:\\z\\nw\\02 - Middle.mp3",
        ],
        "canonical album-track order: missing numbers first, then number, then path"
    );

    // A narrower query keeps only the matching member.
    assert_eq!(
        store
            .album_hit_tracks("Zeta", "New Wave", "opening")
            .expect("album hit tracks")
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\z\\nw\\01 - Opening.mp3"],
        "only the matching track of the album is listed"
    );

    // A track-hit album lists the single matching track.
    assert_eq!(
        store
            .album_hit_tracks("Alpha", "Only", "alpha one")
            .expect("album hit tracks")
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\a\\only\\1.mp3"],
        "the track hit drills to exactly the hit"
    );

    // Unknown albums and no-match queries degrade to empty.
    assert!(
        store
            .album_hit_tracks("Zeta", "New Wave", "zzz")
            .expect("no-match tracks")
            .is_empty()
    );
    assert!(
        store
            .album_hit_tracks("Nobody", "Nothing", "wave")
            .expect("unknown album")
            .is_empty()
    );
}

#[test]
fn test_album_is_name_hit_distinguishes_name_hits_from_track_hits() {
    let (_dir, store) = seeded_entity_hit_store();

    // Title and album-artist matches are name hits.
    assert!(
        store
            .album_is_name_hit("Zeta", "New Wave", "wave")
            .expect("name hit query")
    );
    assert!(
        store
            .album_is_name_hit("Zeta", "New Wave", "zeta")
            .expect("name hit query")
    );
    assert!(
        store
            .album_is_name_hit("Чайковский", "Балеты", "чайковский")
            .expect("name hit query")
    );

    // A track-only match is NOT a name hit.
    assert!(
        !store
            .album_is_name_hit("Zeta", "New Wave", "opening")
            .expect("name hit query")
    );
    assert!(
        !store
            .album_is_name_hit("Alpha", "Only", "alpha one")
            .expect("name hit query")
    );

    // Unknown albums and no-match queries answer false.
    assert!(
        !store
            .album_is_name_hit("Zeta", "New Wave", "zzz")
            .expect("name hit query")
    );
    assert!(
        !store
            .album_is_name_hit("Nobody", "Nothing", "wave")
            .expect("name hit query")
    );
}

#[test]
fn test_artist_is_name_hit_distinguishes_name_hits_from_album_and_track_hits() {
    let (_dir, store) = seeded_entity_hit_store();

    // The artist's own name matching is a name hit, in either case fold.
    assert!(
        store
            .artist_is_name_hit("Zeta", "zeta")
            .expect("name hit query")
    );
    assert!(
        store
            .artist_is_name_hit("Zeta", "ZETA")
            .expect("name hit query")
    );
    // A non-Latin artist name folded by the write-time lowercased column.
    assert!(
        store
            .artist_is_name_hit("Чайковский", "чайковский")
            .expect("name hit query")
    );

    // A member track matching is NOT a name hit.
    assert!(
        !store
            .artist_is_name_hit("Zeta", "opening")
            .expect("name hit query")
    );
    // A member album matching by its own title is NOT a name hit either: the
    // drill must keep to the hit albums for that query.
    assert!(
        !store
            .artist_is_name_hit("Zeta", "wave")
            .expect("name hit query")
    );
    assert!(
        !store
            .artist_is_name_hit("Alpha", "alpha one")
            .expect("name hit query")
    );

    // Unknown artists and no-match queries answer false.
    assert!(
        !store
            .artist_is_name_hit("Zeta", "zzz")
            .expect("name hit query")
    );
    assert!(
        !store
            .artist_is_name_hit("Nobody", "wave")
            .expect("name hit query")
    );
}

/// The genre-hit fixture library: multiple artists and albums with explicit
/// per-track genres, including a semicolon-separated multi-genre track and a
/// name-hit album whose tracks do NOT match the query.
fn genre_hit_fixtures() -> Vec<Track> {
    vec![
        genre_track(
            "f:\\rockers\\hard\\1.mp3",
            "Riff One",
            "Rockers",
            "Hard",
            Some("Rock"),
        ),
        genre_track(
            "f:\\rockers\\hard\\2.mp3",
            "Riff Two",
            "Rockers",
            "Hard",
            Some("Rock"),
        ),
        genre_track(
            "f:\\rockers\\soft\\1.mp3",
            "Ballad",
            "Rockers",
            "Soft",
            Some("Rock"),
        ),
        genre_track(
            "f:\\rockers\\road\\1.mp3",
            "RR1",
            "Rockers",
            "Rocky Road",
            Some("Rock"),
        ),
        genre_track(
            "f:\\jazz\\blue\\1.mp3",
            "Mellow",
            "Jazzy",
            "Blue",
            Some("Jazz"),
        ),
        genre_track(
            "f:\\jazz\\blue\\2.mp3",
            "Smooth",
            "Jazzy",
            "Blue",
            Some("Jazz"),
        ),
        genre_track(
            "f:\\mixed\\both\\1.mp3",
            "Omni",
            "Mixed",
            "Both",
            Some("Rock; Jazz"),
        ),
        genre_track(
            "f:\\mixed\\both\\2.mp3",
            "Pop Hit",
            "Mixed",
            "Both",
            Some("Pop"),
        ),
    ]
}

/// Seed the genre-hit fixture library and return the store.
fn seeded_genre_hit_store() -> (tempfile::TempDir, riff_infra::store::SqliteStore) {
    let (dir, _, mut store, _changes_rx) = open_store_with_path();
    store
        .apply_scan_batch(&genre_hit_fixtures())
        .expect("fixtures apply");
    (dir, store)
}

#[test]
fn test_hit_albums_in_genre_list_albums_that_hold_the_genre_and_hit() {
    let (_dir, store) = seeded_genre_hit_store();

    // A genre-bearing track hit surfaces its album, carrying only the
    // genre-bearing hit tracks.
    let riff = store
        .hit_albums_in_genre("Rock", "riff", 0, 100)
        .expect("genre hit albums query");
    assert_eq!(
        hit_album_keys(&riff),
        [("Rockers", "Hard")],
        "an album holding a hit Rock track appears"
    );
    assert_eq!(
        riff[0]
            .tracks
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\rockers\\hard\\1.mp3", "f:\\rockers\\hard\\2.mp3"],
        "both genre-bearing hit tracks ride along in canonical order"
    );

    // A name-hit album within the genre appears by its own name; its member
    // tracks hit too (the album title lives in each track's search text).
    let rocky = store
        .hit_albums_in_genre("Rock", "rocky", 0, 100)
        .expect("genre hit albums query");
    assert_eq!(
        hit_album_keys(&rocky),
        [("Rockers", "Rocky Road")],
        "a name-hit album within the genre appears (never a dead-end)"
    );
    assert_eq!(
        rocky[0]
            .tracks
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\rockers\\road\\1.mp3"],
        "the name-hit album's member track matches through its title too"
    );

    // Semicolon-separated genre segments match exactly: `Rock; Jazz` holds
    // both genres; a track in a different genre never leaks in.
    assert_eq!(
        hit_album_keys(
            &store
                .hit_albums_in_genre("Rock", "omni", 0, 100)
                .expect("genre hit albums query")
        ),
        [("Mixed", "Both")],
        "a multi-genre track matches each of its segments"
    );
    assert_eq!(
        hit_album_keys(
            &store
                .hit_albums_in_genre("Jazz", "omni", 0, 100)
                .expect("genre hit albums query")
        ),
        [("Mixed", "Both")],
        "the same track matches its other segment"
    );
    assert!(
        store
            .hit_albums_in_genre("Jazz", "riff", 0, 100)
            .expect("genre hit albums query")
            .is_empty(),
        "a Rock-only hit never appears in Jazz"
    );

    // Canonical order within the genre: artist ascending, then year
    // descending with missing years last, then title.
    assert_eq!(
        hit_album_keys(
            &store
                .hit_albums_in_genre("Rock", "r", 0, 100)
                .expect("genre hit albums query")
        ),
        [
            ("Rockers", "Hard"),
            ("Rockers", "Rocky Road"),
            ("Rockers", "Soft"),
        ],
        "canonical browsing order inside the genre (title-ascending on year ties)"
    );
}

#[test]
fn test_hit_albums_in_genre_windows_are_bounded_with_correct_totals() {
    let (_dir, store) = seeded_genre_hit_store();

    assert_eq!(
        hit_album_keys(
            &store
                .hit_albums_in_genre("Rock", "r", 0, 2)
                .expect("window 0")
        ),
        [("Rockers", "Hard"), ("Rockers", "Rocky Road")],
        "the first window takes the first two canonical rows"
    );
    assert_eq!(
        hit_album_keys(
            &store
                .hit_albums_in_genre("Rock", "r", 2, 10)
                .expect("window 2")
        ),
        [("Rockers", "Soft")],
        "the second window takes the remainder"
    );
}

#[test]
fn test_hit_artists_in_genre_list_artists_with_a_genre_scoped_hit_album() {
    let (_dir, store) = seeded_genre_hit_store();

    // The artist behind a genre-scoped hit album appears with only its
    // genre-scoped hit-album keys.
    assert_eq!(
        hit_artist_keys(
            &store
                .hit_artists_in_genre("Rock", "riff", 0, 100)
                .expect("genre hit artists query")
        ),
        [("Rockers".to_string(), vec!["Rockers - Hard".to_string()])],
        "an artist with a genre-matching hit album appears with only its hit-album key"
    );

    // The multi-genre track surfaces its artist under each of its segments.
    assert_eq!(
        hit_artist_keys(
            &store
                .hit_artists_in_genre("Jazz", "omni", 0, 100)
                .expect("genre hit artists query")
        ),
        [("Mixed".to_string(), vec!["Mixed - Both".to_string()])],
        "the semicolon-separated track matches its Jazz segment too"
    );

    // A name-hit album within the genre expands the artist's hit keys.
    assert_eq!(
        hit_artist_keys(
            &store
                .hit_artists_in_genre("Rock", "rocky", 0, 100)
                .expect("genre hit artists query")
        ),
        [(
            "Rockers".to_string(),
            vec!["Rockers - Rocky Road".to_string()]
        )],
        "a name-hit genre album is a genre-scoped hit for its artist"
    );

    // A genre with no hits answers empty.
    assert!(
        store
            .hit_artists_in_genre("Jazz", "riff", 0, 100)
            .expect("genre hit artists query")
            .is_empty()
    );
}

#[test]
fn test_album_hit_tracks_in_genre_filter_by_both_genre_and_query() {
    let (_dir, store) = seeded_genre_hit_store();

    assert_eq!(
        store
            .album_hit_tracks_in_genre("Rockers", "Hard", "Rock", "riff")
            .expect("genre album hit tracks")
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\rockers\\hard\\1.mp3", "f:\\rockers\\hard\\2.mp3"],
        "both Rock tracks hit"
    );
    assert!(
        store
            .album_hit_tracks_in_genre("Rockers", "Hard", "Jazz", "riff")
            .expect("genre album hit tracks")
            .is_empty(),
        "an album without the genre never answers"
    );
    assert_eq!(
        store
            .album_hit_tracks_in_genre("Mixed", "Both", "Jazz", "omni")
            .expect("genre album hit tracks")
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        ["f:\\mixed\\both\\1.mp3"],
        "a semicolon-separated track matches its Jazz segment and the query"
    );
}

#[test]
fn test_hit_genre_counts_aggregate_hit_tracks_across_split_segments() {
    let (_dir, store) = seeded_genre_hit_store();

    // "o" hits tracks in Rock, Jazz (including the `Rock; Jazz` track under
    // BOTH segments) and Pop — name-ascending, hit-track counts.
    assert_eq!(
        store.hit_genre_counts("o").expect("hit genre counts"),
        vec![
            GenreCount {
                genre: "Jazz".to_string(),
                tracks: 3,
            },
            GenreCount {
                genre: "Pop".to_string(),
                tracks: 1,
            },
            GenreCount {
                genre: "Rock".to_string(),
                tracks: 5,
            },
        ],
        "hit-scoped genre counts split semicolon segments exactly like genre_counts"
    );

    // Matching is case-insensitive like every other hit read.
    assert_eq!(
        store.hit_genre_counts("O").expect("hit genre counts"),
        store.hit_genre_counts("o").expect("hit genre counts"),
        "case-insensitive aggregation"
    );

    // No hit tracks aggregate into nothing.
    assert_eq!(
        store
            .hit_genre_counts("zzz-no-match")
            .expect("no-match counts"),
        [] as [GenreCount; 0]
    );
}

// --- Application Store: counts + status read models (design-handoff issue 05) --

#[test]
fn test_library_counts_answers_track_artist_album_and_genre_totals() {
    let (_dir, mut store, _changes_rx) = open_store();

    // A fresh store counts nothing — no phantom rows for the empty sidebar.
    let fresh = store.library_counts().expect("fresh store counts");
    assert_eq!(
        (fresh.tracks, fresh.artists, fresh.albums, fresh.genres),
        (0, 0, 0, 0),
        "a fresh store counts no tracks, artists, albums, or genres"
    );

    // 5 tracks; Alpha holds two albums, Beta one; Rock and Jazz are the only
    // genres (the empty-tag track aggregates into no genre row).
    store
        .apply_scan_batch(&[
            genre_track("f:\\c\\1.mp3", "One", "Alpha", "LP", Some("Rock")),
            genre_track("f:\\c\\2.mp3", "Two", "Alpha", "LP", Some("Rock")),
            genre_track("f:\\c\\3.mp3", "Three", "Alpha", "B-Sides", Some("Rock")),
            genre_track("f:\\c\\4.mp3", "Four", "Beta", "EP", Some("Jazz")),
            genre_track("f:\\c\\5.mp3", "Five", "Beta", "EP", Some("")),
        ])
        .expect("batch applies");

    let counts = store.library_counts().expect("library counts query");
    assert_eq!(counts.tracks, 5, "total track count");
    assert_eq!(counts.artists, 2, "distinct album artists");
    assert_eq!(counts.albums, 3, "distinct (album artist, title) albums");
    assert_eq!(counts.genres, 2, "distinct non-empty per-track genres");
}

/// A smart-list fixture with explicit history control: `favorite`,
/// `play_count`, `last_played`, and `date_added` decide every smart list's
/// membership, exactly like the SQL smart-list queries do. (Distinct from
/// the smart-playlist section's `smart_track`, whose second parameter is a
/// title.)
fn count_track(
    path: &str,
    title: &str,
    favorite: bool,
    play_count: u32,
    last_played: Option<SystemTime>,
    date_added: Option<SystemTime>,
) -> Track {
    Track {
        favorite,
        play_count,
        last_played,
        date_added,
        ..browsing_track(path, title, "Alpha", "LP", None, None)
    }
}

#[test]
fn test_smart_list_counts_match_the_smart_list_membership() {
    let (_dir, mut store, _changes_rx) = open_store();

    // A fresh store counts nothing for every smart list.
    assert!(
        store
            .smart_list_counts()
            .expect("fresh store smart list counts")
            .iter()
            .all(|(_, count)| *count == 0),
        "a fresh store has empty smart lists"
    );

    let now = SystemTime::now();
    let long_ago = now - LOST_GEMS_THRESHOLD * 3;
    // Scan batches persist metadata + `date_added` but deliberately never
    // touch history columns; favorites and play history are seeded exactly
    // like the smart-playlist tests seed them.
    let fixtures = [
        // Favorite, played just now, added just now: qualifies for
        // Favorites, Most Played, Recently Played, Recently Added.
        count_track("f:\\s\\1.mp3", "One", true, 2, Some(now), Some(now)),
        // Never played, added just now: Recently Added + Never Played.
        count_track("f:\\s\\2.mp3", "Two", false, 0, None, Some(now)),
        // Played once, long ago: Most Played, Recently Played (any play
        // stamp qualifies), Lost Gems (older than the threshold), and
        // Recently Added.
        count_track("f:\\s\\3.mp3", "Three", false, 1, Some(long_ago), Some(now)),
    ];
    store.apply_scan_batch(&fixtures).expect("batch applies");

    for track in &fixtures {
        let last_nanos = track.last_played.map(|t| {
            i64::try_from(t.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()).unwrap()
        });
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE tracks SET play_count = ?1, last_played_nanos = ?2 WHERE path = ?3",
                    rusqlite::params![i64::from(track.play_count), last_nanos, track.id.0],
                )
            })
            .expect("seeding history must work");
        if track.favorite {
            use riff_persistence::store::LibraryMutationStore as _;
            store
                .set_track_favorite(&track.id, true)
                .expect("seeding favorite must work");
        }
    }

    let counts = store.smart_list_counts().expect("smart list counts");
    assert_eq!(
        counts,
        vec![
            (SmartPlaylistKind::Favorites, 1),
            (SmartPlaylistKind::RecentlyAdded, 3),
            (SmartPlaylistKind::MostPlayed, 2),
            (SmartPlaylistKind::RecentlyPlayed, 2),
            (SmartPlaylistKind::NeverPlayed, 1),
            // The never-played track qualifies for Lost Gems too — unheard
            // tracks are gems by definition.
            (SmartPlaylistKind::LostGems, 2),
        ],
        "per-smart-list totals in stable SmartPlaylistKind::ALL order"
    );
}

#[test]
fn test_folder_track_count_scopes_to_the_component_wise_subtree() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);

    // The fixture library holds 4 tracks directly under "a", 1 under "ab"
    // (the sibling-prefix trap: "a" must not match "ab"/"a_b"/"axb"/"a#b"),
    // and 1 under "deep/only".
    let base = dir.path();
    assert_eq!(
        store
            .folder_track_count(&base.join("a"))
            .expect("count under a"),
        4,
        "every track under the folder counts, not just direct children"
    );
    assert_eq!(
        store
            .folder_track_count(&base.join("ab"))
            .expect("count under ab"),
        1,
        "a sibling sharing a byte-prefix never matches"
    );
    assert_eq!(
        store
            .folder_track_count(&base.join("deep").join("only"))
            .expect("count under deep/only"),
        1
    );
    assert_eq!(
        store
            .folder_track_count(base)
            .expect("count under the root"),
        10,
        "the root counts the whole subtree"
    );
    assert_eq!(
        store
            .folder_track_count(&base.join("nowhere"))
            .expect("count under an unknown folder"),
        0,
        "an unknown folder counts nothing"
    );
}

#[test]
fn test_last_full_scan_round_trips_and_records_as_a_library_mutation() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let (tx, rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let mut store = SqliteStore::open_and_migrate(tmp.path(), tx).unwrap();

    // A fresh store has never scanned — no timestamp, no phantom epoch.
    assert_eq!(
        store.last_full_scan().expect("fresh store read"),
        None,
        "a fresh store has no last-scan timestamp"
    );

    let first = FullScanSummary {
        at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        files: 10,
        errors: 1,
    };
    store
        .record_full_scan_completed(first)
        .expect("recording a scan must commit");
    assert_eq!(
        store.last_full_scan().expect("read after first record"),
        Some(first),
        "the recorded summary round-trips through the store"
    );

    // Recording again overwrites: the value answers for the LAST full scan.
    let second = FullScanSummary {
        at: first.at + Duration::from_mins(1),
        files: 20,
        errors: 0,
    };
    store
        .record_full_scan_completed(second)
        .expect("recording again must commit");
    assert_eq!(
        store.last_full_scan().expect("read after second record"),
        Some(second),
        "a later scan overwrites the earlier summary"
    );

    // Recording is a committed Library mutation: each commit bumps the
    // session Library generation and emits exactly one Library event beside
    // the bump, so the read models refetch after a scan.
    let gen_before = store.library_generation().current();
    let third = FullScanSummary {
        at: second.at + Duration::from_mins(1),
        files: 30,
        errors: 2,
    };
    store
        .record_full_scan_completed(third)
        .expect("third record must commit");
    assert_eq!(
        store.library_generation().current(),
        gen_before + 1,
        "recording a scan bumps the Library generation"
    );
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    assert_eq!(
        events,
        vec![
            StoreChanged::Library(1),
            StoreChanged::Library(2),
            StoreChanged::Library(3),
        ],
        "one Library event per recorded scan, carrying the new generation"
    );
}

// --- Application Store: folder navigation queries (ticket 08) ---------------

/// A folder-tree fixture library under a REAL temporary directory: the
/// former in-memory `subdirs_with_audio` stats candidate children with
/// `is_dir()`, so parity fixtures must exist on disk. Exercises nested
/// paths, the sibling-prefix trap ("a" alongside "ab"), percent/underscore/
/// escape-character names, a directory containing only subdirectories, and
/// direct-track ordering with missing and duplicate numbers.
fn folder_fixtures(base: &std::path::Path) -> Vec<Track> {
    for dir in ["a", "ab", "50% Off", "a_b", "axb", "a#b", "deep/only"] {
        std::fs::create_dir_all(base.join(dir)).expect("fixture dirs must create");
    }

    let track = |rel: &str, title: &str, number: Option<u32>| {
        // Fixture literals use `\` for readability; translate to the platform
        // separator so stored paths match real scan output on every OS
        // (`Path::join` only normalizes separators native to the host).
        let path = base.join(rel.replace('\\', std::path::MAIN_SEPARATOR_STR));
        browsing_track(
            &path.to_string_lossy(),
            title,
            "Artist",
            "Album",
            number,
            Some(2001),
        )
    };

    vec![
        track("a\\zz unnumbered.mp3", "Unnumbered", None),
        track("a\\05 beta.mp3", "Beta", Some(5)),
        track("a\\01 alpha.mp3", "Alpha", Some(1)),
        track("a\\05 alpha.mp3", "Alpha Five", Some(5)),
        track("ab\\three.mp3", "Three", Some(1)),
        track("50% Off\\hit.mp3", "Hit", Some(1)),
        track("a_b\\under.mp3", "Under", Some(1)),
        track("axb\\wildcard victim.mp3", "Victim", Some(1)),
        track("a#b\\hash.mp3", "Hash", Some(1)),
        track("deep\\only\\nested.mp3", "Nested", Some(1)),
    ]
}

fn seeded_folder_store(dir: &tempfile::TempDir) -> riff_infra::store::SqliteStore {
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let mut store = riff_infra::store::SqliteStore::open_and_migrate(
        &dir.path().join("riff.sqlite3"),
        changes_tx,
    )
    .expect("fresh store must open");
    store
        .apply_scan_batch(&folder_fixtures(dir.path()))
        .expect("fixtures apply");
    store
}

/// An independent Rust-only restatement of the former in-memory folder
/// logic, computed straight from the fixture list with pure `Path` ops
/// (including the `is_dir()` stats the original performed). The SQL queries
/// must agree with this without going through any shared code.
mod folder_reference {
    use super::*;
    use std::path::Path;

    pub fn has_audio(tracks: &[Track], folder: &Path) -> bool {
        tracks.iter().any(|t| t.file_path.starts_with(folder))
    }

    pub fn tree_ids(tracks: &[Track], folder: &Path) -> Vec<TrackId> {
        let mut ids: Vec<TrackId> = tracks
            .iter()
            .filter(|t| t.file_path.starts_with(folder))
            .map(|t| t.id.clone())
            .collect();
        ids.sort_by(|a, b| {
            let path_of = |id: &TrackId| tracks.iter().find(|t| &t.id == id).map(|t| &t.file_path);
            path_of(a).cmp(&path_of(b))
        });
        ids
    }

    pub fn direct_ids(tracks: &[Track], folder: &Path) -> Vec<TrackId> {
        let mut direct: Vec<&Track> = tracks
            .iter()
            .filter(|t| t.file_path.parent() == Some(folder))
            .collect();
        direct.sort_by(|a, b| {
            a.metadata
                .track_number
                .unwrap_or(0)
                .cmp(&b.metadata.track_number.unwrap_or(0))
                .then_with(|| a.file_path.file_name().cmp(&b.file_path.file_name()))
        });
        direct.into_iter().map(|t| t.id.clone()).collect()
    }

    pub fn subdirs(tracks: &[Track], folder: &Path) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for track in tracks {
            let track_path = &track.file_path;
            if !track_path.starts_with(folder) {
                continue;
            }
            let Ok(relative) = track_path.strip_prefix(folder) else {
                continue;
            };
            if let Some(first_component) = relative.iter().next() {
                let child_dir = folder.join(first_component);
                if child_dir.is_dir() && seen.insert(child_dir.clone()) {
                    dirs.push(child_dir);
                }
            }
        }
        dirs.sort();
        dirs
    }
}

#[test]
fn test_folder_queries_match_independent_reference_logic() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let fixtures = folder_fixtures(dir.path());

    // Probe folders: roots, mid-level, leaves, special-char names, the
    // sibling-prefix trap pair, a file path used as a folder, and a folder
    // that does not exist anywhere in the library. Relative parts use the
    // platform separator, exactly like every real caller (scan joins and
    // `Path::join` never mix separators).
    let rel_probes = [
        "",
        "a",
        "ab",
        "50% Off",
        "a_b",
        "axb",
        "a#b",
        "deep",
        std::path::MAIN_SEPARATOR_STR,
    ];
    let mut probes: Vec<std::path::PathBuf> =
        rel_probes.iter().map(|rel| dir.path().join(rel)).collect();
    probes.push(dir.path().join("deep").join("only"));
    probes.push(dir.path().join("a").join("01 alpha.mp3"));

    for probe in probes {
        let label = probe.to_string_lossy();

        assert_eq!(
            store.folder_has_audio(&probe).expect("has_audio"),
            folder_reference::has_audio(&fixtures, &probe),
            "folder_has_audio({label}) must match the reference logic"
        );

        assert_eq!(
            store.track_ids_in_folder_tree(&probe).expect("tree ids"),
            folder_reference::tree_ids(&fixtures, &probe),
            "track_ids_in_folder_tree({label}) must match the reference logic"
        );

        assert_eq!(
            store
                .tracks_in_folder(&probe)
                .expect("direct tracks")
                .iter()
                .map(|t| t.id.clone())
                .collect::<Vec<_>>(),
            folder_reference::direct_ids(&fixtures, &probe),
            "tracks_in_folder({label}) must match the reference order and membership"
        );

        assert_eq!(
            store.subdirs_with_audio(&probe).expect("subdirs"),
            folder_reference::subdirs(&fixtures, &probe),
            "subdirs_with_audio({label}) must match the reference logic"
        );
    }
}

#[test]
fn test_folder_prefix_queries_escape_like_wildcards() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    // "%" in a folder name is a literal character: querying it must not
    // degrade into a wildcard match.
    let off = base.join("50% Off");
    assert!(
        store.folder_has_audio(&off).expect("has_audio"),
        "the literal percent folder contains audio"
    );
    assert_eq!(
        store
            .track_ids_in_folder_tree(&off)
            .expect("tree ids")
            .len(),
        1,
        "only the percent folder's own track matches"
    );

    // "_" in a folder name must not match sibling "axb".
    let under = base.join("a_b");
    assert!(store.folder_has_audio(&under).expect("has_audio"));
    assert_eq!(
        store.track_ids_in_folder_tree(&under).expect("tree ids"),
        vec![TrackId(
            base.join("a_b")
                .join("under.mp3")
                .to_string_lossy()
                .into_owned()
        )],
        "underscore must be literal, not a single-char wildcard"
    );

    // The escape character itself appearing in a name stays literal.
    let hash = base.join("a#b");
    assert!(store.folder_has_audio(&hash).expect("has_audio"));
    assert_eq!(
        store.track_ids_in_folder_tree(&hash).expect("tree ids"),
        vec![TrackId(
            base.join("a#b")
                .join("hash.mp3")
                .to_string_lossy()
                .into_owned()
        )],
        "escape characters in names are matched literally"
    );
}

#[test]
fn test_sibling_prefix_trap_is_not_matched() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    // Folder "a" alongside "ab": prefix matching must respect component
    // boundaries, so "ab"'s subtree never leaks into "a".
    let a = base.join("a");
    let mut ids = store.track_ids_in_folder_tree(&a).expect("tree ids");
    ids.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        ids,
        vec![
            TrackId(
                base.join("a")
                    .join("01 alpha.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
            TrackId(
                base.join("a")
                    .join("05 alpha.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
            TrackId(
                base.join("a")
                    .join("05 beta.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
            TrackId(
                base.join("a")
                    .join("zz unnumbered.mp3")
                    .to_string_lossy()
                    .into_owned()
            ),
        ],
        "folder a holds exactly its own four tracks"
    );

    let ab = base.join("ab");
    assert_eq!(
        store.track_ids_in_folder_tree(&ab).expect("tree ids"),
        vec![TrackId(
            base.join("ab")
                .join("three.mp3")
                .to_string_lossy()
                .into_owned()
        )],
        "folder ab holds exactly its own track"
    );
}

#[test]
fn test_subdirs_with_audio_lists_only_direct_child_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    let children = store.subdirs_with_audio(base).expect("subdirs");
    let names: Vec<String> = children
        .iter()
        .map(|c| c.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec!["50% Off", "a", "a#b", "a_b", "ab", "axb", "deep"],
        "every child with audio lists once, name-ascending; direct files never appear"
    );

    // A directory containing only subdirectories still expands.
    let deep = base.join("deep");
    assert_eq!(
        store.subdirs_with_audio(&deep).expect("subdirs"),
        vec![base.join("deep").join("only")],
        "directories holding only subdirectories list their child"
    );
    assert!(
        store
            .tracks_in_folder(&deep)
            .expect("direct tracks")
            .is_empty(),
        "deep has no direct tracks of its own"
    );
}

#[test]
fn test_tracks_in_folder_orders_number_then_filename_missing_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let a = dir.path().join("a");

    let tracks = store.tracks_in_folder(&a).expect("direct tracks");
    assert_eq!(
        tracks.iter().map(|t| t.id.0.as_str()).collect::<Vec<_>>(),
        [
            a.join("zz unnumbered.mp3").to_string_lossy(), // missing number first
            a.join("01 alpha.mp3").to_string_lossy(),
            a.join("05 alpha.mp3").to_string_lossy(), // tie by filename
            a.join("05 beta.mp3").to_string_lossy(),
        ],
        "direct tracks sort by number then filename, missing numbers first"
    );
}

#[test]
fn test_folder_has_search_match_scopes_to_subtree() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded_folder_store(&dir);
    let base = dir.path();

    // "victim" only exists under axb.
    let a = base.join("a");
    let axb = base.join("axb");
    assert!(
        !store
            .folder_has_search_match(&a, "victim")
            .expect("search match"),
        "sibling subtrees must not satisfy the search filter"
    );
    assert!(
        store
            .folder_has_search_match(&axb, "victim")
            .expect("search match"),
        "the owning subtree satisfies the search filter"
    );
    assert!(
        store
            .folder_has_search_match(&a, "unnumbered")
            .expect("search match"),
        "a real match inside the subtree reports true"
    );
}

// --- Application Store: smart playlists as SQL (ticket 09) ------------------

use riff_persistence::track::SmartPlaylistKind;

/// A fixture track with full control over history and dates.
fn smart_track(
    path: &str,
    title: Option<&str>,
    play_count: u32,
    last_played: Option<SystemTime>,
    date_added: Option<SystemTime>,
) -> Track {
    Track {
        id: TrackId::from_path(std::path::Path::new(path)),
        file_path: PathBuf::from(path),
        metadata: TrackMetadata {
            title: title.map(str::to_string),
            artist: Some("Artist".to_string()),
            album: Some("Album".to_string()),
            album_artist: Some("Artist".to_string()),
            ..TrackMetadata::default()
        },
        duration: None,
        sample_rate: None,
        channels: None,
        play_count,
        last_played,
        date_added,
        favorite: false,
        search_text: String::new(),
    }
}

/// The fixture library, shared by the store seeding and the independent
/// smart-playlist reference oracle.
fn smart_fixtures() -> Vec<Track> {
    let now = SystemTime::now();
    let days_ago = |days: u64| now - Duration::from_secs(days * 86_400);
    let epoch_plus = |secs: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);

    vec![
        smart_track(
            "f:\\sm\\zebra.mp3",
            Some("Zebra"),
            10,
            Some(days_ago(100)),
            Some(epoch_plus(5_000)),
        ),
        // Count tie broken by display-title fallback stems ("alpha" first).
        smart_track(
            "f:\\sm\\alpha.mp3",
            None,
            3,
            Some(days_ago(200)),
            Some(epoch_plus(9_000)),
        ),
        // Stem-vs-path divergence pair: titles "a.b" vs "a" order opposite
        // to raw path bytes, pinning the exact comparator.
        smart_track(
            "f:\\sm\\a.b.mp3",
            None,
            3,
            Some(days_ago(300)),
            Some(epoch_plus(1_000)),
        ),
        smart_track(
            "f:\\sm\\a.mp3",
            None,
            3,
            Some(days_ago(310)),
            Some(epoch_plus(2_000)),
        ),
        // Unplayed pair for NeverPlayed's path ordering.
        smart_track(
            "f:\\sm\\never1.mp3",
            Some("Never One"),
            0,
            None,
            Some(epoch_plus(6_000)),
        ),
        smart_track(
            "f:\\sm\\never0.mp3",
            Some("Never Zero"),
            0,
            None,
            Some(epoch_plus(7_000)),
        ),
        // Missing date_added: excluded from Recently Added entirely.
        smart_track("f:\\sm\\nodate.mp3", Some("No Date"), 0, None, None),
        // Future last-played counts as a Lost Gem ("very old").
        smart_track(
            "f:\\sm\\future.mp3",
            Some("Future"),
            1,
            Some(now + Duration::from_hours(1)),
            Some(epoch_plus(8_000)),
        ),
        // Recently-added tie on identical date_added values.
        smart_track(
            "f:\\sm\\tie_b.mp3",
            Some("Tie B"),
            0,
            None,
            Some(epoch_plus(4_000)),
        ),
        smart_track(
            "f:\\sm\\tie_a.mp3",
            Some("Tie A"),
            0,
            None,
            Some(epoch_plus(4_000)),
        ),
        // Played too recently to be a gem.
        smart_track(
            "f:\\sm\\recent_play.mp3",
            Some("Recent Play"),
            2,
            Some(days_ago(30)),
            Some(epoch_plus(3_000)),
        ),
    ]
}

/// Seed the fixture library: scan batches persist metadata + `date_added`;
/// play history is written through direct updates exactly like the
/// tag-refresh tests do (the scan upsert deliberately never touches
/// history columns).
fn seeded_smart_store(db_path: &std::path::Path) -> riff_infra::store::SqliteStore {
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let mut store = riff_infra::store::SqliteStore::open_and_migrate(db_path, changes_tx)
        .expect("fresh store must open");

    let fixtures = smart_fixtures();

    store.apply_scan_batch(&fixtures).expect("fixtures apply");

    for track in &fixtures {
        let last_nanos =
            track
                .last_played
                .map(|t| match t.duration_since(SystemTime::UNIX_EPOCH) {
                    Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
                    Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
                });
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE tracks SET play_count = ?1, last_played_nanos = ?2 WHERE path = ?3",
                    rusqlite::params![i64::from(track.play_count), last_nanos, track.id.0],
                )
            })
            .expect("seeding history must work");
    }
    store
}

/// An independent Rust-only restatement of the four smart-playlist
/// computations, straight from the fixture list with the former comparators.
/// The SQL implementations must agree with this without going through any
/// shared code. Exercising [`LOST_GEMS_THRESHOLD`] here also pins the
/// relocated constant to the semantics it parameterizes.
mod smart_reference {
    use super::*;
    use riff_persistence::store::LOST_GEMS_THRESHOLD;

    pub fn playlist(tracks: &[Track], kind: SmartPlaylistKind, limit: usize) -> Vec<TrackId> {
        let mut selected: Vec<&Track> = match kind {
            SmartPlaylistKind::Favorites => {
                let mut favorites: Vec<&Track> = tracks.iter().filter(|t| t.favorite).collect();
                favorites.sort_by(|a, b| a.file_path.cmp(&b.file_path));
                favorites
            }
            SmartPlaylistKind::RecentlyAdded => {
                // Sorted by the stored `date_added`, deliberately NOT the
                // filesystem mtime; newest first, path tiebreak.
                let mut dated: Vec<(SystemTime, &Track)> = tracks
                    .iter()
                    .filter_map(|t| t.date_added.map(|added| (added, t)))
                    .collect();
                dated.sort_by(|(added_a, a), (added_b, b)| {
                    added_b
                        .cmp(added_a)
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                dated.into_iter().map(|(_, t)| t).collect()
            }
            SmartPlaylistKind::RecentlyPlayed => {
                // Newest finished-play first; never-played rows are excluded
                // outright. Ties break by path for a stable order.
                let mut played: Vec<(SystemTime, &Track)> = tracks
                    .iter()
                    .filter_map(|t| t.last_played.map(|last| (last, t)))
                    .collect();
                played.sort_by(|(last_a, a), (last_b, b)| {
                    last_b
                        .cmp(last_a)
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                played.into_iter().map(|(_, t)| t).collect()
            }
            SmartPlaylistKind::MostPlayed => {
                let mut played: Vec<&Track> = tracks.iter().filter(|t| t.play_count > 0).collect();
                played.sort_by(|a, b| {
                    b.play_count
                        .cmp(&a.play_count)
                        .then_with(|| {
                            a.metadata
                                .display_title(&a.file_path)
                                .cmp(&b.metadata.display_title(&b.file_path))
                        })
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                played
            }
            SmartPlaylistKind::NeverPlayed => {
                let mut unplayed: Vec<&Track> =
                    tracks.iter().filter(|t| t.play_count == 0).collect();
                unplayed.sort_by(|a, b| a.file_path.cmp(&b.file_path));
                unplayed
            }
            SmartPlaylistKind::LostGems => {
                // Unheard for 90+ days: last played older than the threshold
                // OR never played at all. Once-played gems come first,
                // longest-unheard first; never-played tracks fill the tail in
                // path order.
                let mut gems: Vec<(SystemTime, &Track)> = tracks
                    .iter()
                    .filter_map(|t| t.last_played.map(|last| (last, t)))
                    .filter(|(last, _)| match last.elapsed() {
                        Ok(age) => age > LOST_GEMS_THRESHOLD,
                        // A clock anomaly (last_played in the future) counts
                        // as "very old" rather than excluding the track.
                        Err(_) => true,
                    })
                    .collect();
                gems.sort_by(|(last_a, a), (last_b, b)| {
                    last_a
                        .cmp(last_b)
                        .then_with(|| a.file_path.cmp(&b.file_path))
                });
                let mut unheard: Vec<&Track> =
                    tracks.iter().filter(|t| t.last_played.is_none()).collect();
                unheard.sort_by(|a, b| a.file_path.cmp(&b.file_path));
                gems.into_iter().map(|(_, t)| t).chain(unheard).collect()
            }
        };
        selected.truncate(limit);
        selected.into_iter().map(|t| t.id.clone()).collect()
    }
}

#[test]
fn test_smart_playlists_match_independent_reference_logic() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let store = seeded_smart_store(&db_path);
    let fixtures = smart_fixtures();

    for kind in [
        SmartPlaylistKind::MostPlayed,
        SmartPlaylistKind::RecentlyAdded,
        SmartPlaylistKind::RecentlyPlayed,
        SmartPlaylistKind::NeverPlayed,
        SmartPlaylistKind::LostGems,
    ] {
        for limit in [usize::MAX, 3] {
            assert_eq!(
                store
                    .smart_playlist(kind, limit)
                    .expect("smart playlist query")
                    .iter()
                    .map(|t| t.id.clone())
                    .collect::<Vec<_>>(),
                smart_reference::playlist(&fixtures, kind, limit),
                "smart_playlist({kind:?}, {limit}) must match the reference logic"
            );
        }
    }

    // Spot-check the interesting shapes so a two-sided bug cannot hide.
    let most_played = store
        .smart_playlist(SmartPlaylistKind::MostPlayed, usize::MAX)
        .expect("most played");
    assert_eq!(
        most_played[0].metadata.title.as_deref(),
        Some("Zebra"),
        "highest play count leads"
    );
    let stems: Vec<Option<&str>> = most_played[1..4]
        .iter()
        .map(|t| t.id.0.split('\\').next_back())
        .collect();
    assert_eq!(
        stems,
        [Some("a.mp3"), Some("a.b.mp3"), Some("alpha.mp3")],
        "count ties break by display-title stem order, not raw path bytes"
    );

    let gems = store
        .smart_playlist(SmartPlaylistKind::LostGems, usize::MAX)
        .expect("lost gems");
    let names: Vec<String> = gems
        .iter()
        .map(|t| t.id.0.split('\\').next_back().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        vec![
            "a.mp3",   // oldest gem leads (310 days unheard)
            "a.b.mp3", // gems sort by last-played ascending, not by path
            "alpha.mp3",
            "zebra.mp3",  // 100 days unheard
            "future.mp3", // future last-played qualifies and sorts last among gems
            "never0.mp3", // unheard tail is path-ordered after all gems
            "never1.mp3",
            "nodate.mp3",
            "tie_a.mp3",
            "tie_b.mp3",
        ],
        "longest-unheard gems first, then never-played in path order; \
         recently played tracks are excluded"
    );
}

#[test]
fn test_recently_played_orders_newest_first_excludes_never_played_and_bounds() {
    let (_dir, _, mut store, _changes_rx) = open_store_with_path();

    let at = |secs: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
    let fixtures = vec![
        smart_track(
            "f:\\rp\\old.mp3",
            Some("Old"),
            1,
            Some(at(1_000)),
            Some(at(10)),
        ),
        // A last-played tie broken by path: "tie_a" leads despite sorting
        // after "tie_b" by insertion.
        smart_track(
            "f:\\rp\\tie_b.mp3",
            Some("Tie B"),
            1,
            Some(at(2_000)),
            Some(at(20)),
        ),
        smart_track(
            "f:\\rp\\tie_a.mp3",
            Some("Tie A"),
            1,
            Some(at(2_000)),
            Some(at(30)),
        ),
        smart_track(
            "f:\\rp\\newest.mp3",
            Some("Newest"),
            1,
            Some(at(3_000)),
            Some(at(40)),
        ),
        smart_track("f:\\rp\\never.mp3", Some("Never"), 0, None, Some(at(50))),
    ];
    store.apply_scan_batch(&fixtures).expect("fixtures apply");
    for track in &fixtures {
        let last_nanos =
            track
                .last_played
                .map(|t| match t.duration_since(SystemTime::UNIX_EPOCH) {
                    Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
                    Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
                });
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE tracks SET play_count = ?1, last_played_nanos = ?2 WHERE path = ?3",
                    rusqlite::params![i64::from(track.play_count), last_nanos, track.id.0],
                )
            })
            .expect("seeding history must work");
    }

    let titles = |tracks: &[Track]| {
        tracks
            .iter()
            .map(|t| t.metadata.title.as_deref().unwrap().to_string())
            .collect::<Vec<_>>()
    };

    let all = store
        .smart_playlist(SmartPlaylistKind::RecentlyPlayed, usize::MAX)
        .expect("recently played");
    assert_eq!(
        titles(&all),
        vec!["Newest", "Tie A", "Tie B", "Old"],
        "most recent last-played leads, ties break by path, never-played excluded"
    );

    // The limit truncates the fully ordered list.
    let bounded = store
        .smart_playlist(SmartPlaylistKind::RecentlyPlayed, 2)
        .expect("bounded recently played");
    assert_eq!(titles(&bounded), vec!["Newest", "Tie A"]);
}

#[test]
fn test_smart_playlists_reflect_committed_mutations_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = seeded_smart_store(&db_path);

    let victim = TrackId("f:\\sm\\never0.mp3".to_string());
    let before: Vec<String> = store
        .smart_playlist(SmartPlaylistKind::NeverPlayed, usize::MAX)
        .expect("never played")
        .iter()
        .map(|t| t.id.0.clone())
        .collect();
    assert!(before.contains(&victim.0), "sanity: unplayed before");

    // One committed play through the mutation port...
    let played_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_234_567);
    assert!(
        store
            .record_track_played(&victim, played_at)
            .expect("recording works"),
        "known track records"
    );

    // ...is visible to the very next smart-playlist query, no restart.
    let after_never: Vec<String> = store
        .smart_playlist(SmartPlaylistKind::NeverPlayed, usize::MAX)
        .expect("never played")
        .iter()
        .map(|t| t.id.0.clone())
        .collect();
    assert!(
        !after_never.contains(&victim.0),
        "played tracks leave Never Played immediately"
    );

    let most_played: Vec<(String, u32)> = store
        .smart_playlist(SmartPlaylistKind::MostPlayed, usize::MAX)
        .expect("most played")
        .iter()
        .map(|t| (t.id.0.clone(), t.play_count))
        .collect();
    let entry = most_played
        .iter()
        .find(|(id, _)| id == &victim.0)
        .expect("played track joins Most Played immediately");
    assert_eq!(entry.1, 1, "the recorded count is what queries see");
}

// --- Application Store: Clear Library (ticket 10) ---------------------------

/// Seed a full store: three tracks with play history across two albums and
/// two artists, a playlist with entries (one pointing at a track), and
/// every kind of setting.
fn seeded_clear_store(db_path: &std::path::Path) -> riff_infra::store::SqliteStore {
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let mut store = riff_infra::store::SqliteStore::open_and_migrate(db_path, changes_tx)
        .expect("fresh store must open");

    let mut t1 = browsing_track(
        "f:\\cl\\a\\one.mp3",
        "One",
        "Alpha",
        "First",
        Some(1),
        Some(2001),
    );
    t1.date_added = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000));
    let mut t2 = browsing_track(
        "f:\\cl\\a\\two.mp3",
        "Two",
        "Alpha",
        "First",
        Some(2),
        Some(2001),
    );
    t2.date_added = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(2_000));
    let t3 = browsing_track(
        "f:\\cl\\b\\three.mp3",
        "Three",
        "Beta",
        "Second",
        Some(1),
        Some(1999),
    );

    store
        .apply_scan_batch(&[t1, t2, t3])
        .expect("fixtures apply");
    // Play history: the wipe must take these derived facts with the tracks.
    store
        .with_connection(|conn| {
            conn.execute(
                "UPDATE tracks SET play_count = 4, last_played_nanos = 42
                 WHERE path = 'f:\\cl\\a\\one.mp3'",
                [],
            )
        })
        .expect("seeding history must work");

    let pid = store
        .create_playlist(
            "Curation",
            &[
                TrackId("f:\\cl\\a\\one.mp3".to_string()),
                TrackId("f:\\cl\\gone.mp3".to_string()),
            ],
        )
        .expect("playlist creation works");
    let _ = pid;

    store
        .save_scalars(&riff_persistence::store::ScalarSettings {
            volume: Some(0.75),
            advanced_mode: true,
            high_contrast: false,
            replaygain_enabled: true,
            ..Default::default()
        })
        .expect("scalars save");
    store
        .save_library_paths(&[PathBuf::from("f:\\cl")])
        .expect("paths save");
    let mut watches = std::collections::HashMap::new();
    watches.insert(
        PathBuf::from("f:\\cl"),
        riff_persistence::store::WatchState::Enabled,
    );
    store.save_watch_states(&watches).expect("watches save");

    // One Custom Smart Playlist with a two-level tree: the maintenance wipe is a
    // Library-collection wipe, and a list's Rule tree is curation, so all three
    // rule tables have to be standing when it finishes.
    store
        .save_smart_playlist(None, "Cleared-Around Me", &custom_rule_tree())
        .expect("the Custom Smart Playlist saves");

    store
}

/// A Custom Smart Playlist tree with a nested Rule Box, for the tests that have
/// to prove the rule tables survive something.
pub fn custom_rule_tree() -> riff_persistence::store::SmartPlaylistTree {
    use riff_persistence::rule::{
        Combine, Comparison, MatchableField, RootBox, Rule, RuleBox, RuleChild, RuleValue,
    };
    SmartPlaylistTree::new(RootBox::new(RuleBox::new(Combine::All).with_children(
        vec![
            RuleChild::Rule(Rule::new(
                MatchableField::Genre,
                Comparison::Contains,
                Some(RuleValue::Text("rock".to_string())),
            )),
            RuleChild::Box(
                RuleBox::new(Combine::Any).with_children(vec![RuleChild::Rule(Rule::new(
                    MatchableField::Year,
                    Comparison::GreaterThan,
                    Some(RuleValue::Number(1990)),
                ))]),
            ),
        ],
    )))
}

/// The three Custom Smart Playlist tables, asserted intact after a maintenance
/// wipe: a list's Rule tree is curation, not Library collection data, so
/// `clear_library` never reaches any of the three.
fn assert_custom_rule_tables_survive(store: &riff_infra::store::SqliteStore) {
    // The three Custom Smart Playlist tables survive the wipe untouched: a list's
    // Rule tree is curation, not collection data, and `clear_library` is the
    // Library collection's own maintenance action.
    assert_eq!(
        store
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM smart_playlists", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .expect("counting lists must work"),
        1,
        "the Custom Smart Playlist row survives the wipe"
    );
    assert_eq!(
        store
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM smart_rule_boxes", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .expect("counting boxes must work"),
        2,
        "its Root Box and its nested box both survive"
    );
    assert_eq!(
        store
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM smart_rules", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .expect("counting rules must work"),
        2,
        "and both Rules — the wipe never reaches the rule tables"
    );
    let custom_id = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT id FROM smart_playlists WHERE name = 'Cleared-Around Me'",
                [],
                |row| row.get::<_, String>(0),
            )
        })
        .expect("reading the list id must work");
    assert_eq!(
        store
            .smart_playlist_tree(&PlaylistId(custom_id))
            .expect("the tree still reads")
            .expect("the list is still there"),
        custom_rule_tree(),
        "and the tree reads back exactly as it was saved, after the wipe"
    );
}

#[test]
fn test_clear_library_wipes_collection_and_preserves_curation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = seeded_clear_store(&db_path);

    let removed = store.clear_library().expect("clear works");
    assert_eq!(removed, 3, "every track row is wiped in one action");

    // Collection tables are empty; queries see an empty library immediately.
    assert_eq!(track_count(&store), 0);
    assert!(
        tracks_window(&store, 0, 50).is_empty(),
        "flat list is empty without a restart"
    );
    assert!(
        store.all_artists().expect("artists").is_empty(),
        "browse view is empty"
    );
    assert!(
        !store
            .folder_has_audio(std::path::Path::new("f:\\cl"))
            .expect("folder probe"),
        "folder views see no audio"
    );
    for kind in [
        SmartPlaylistKind::MostPlayed,
        SmartPlaylistKind::RecentlyAdded,
        SmartPlaylistKind::RecentlyPlayed,
        SmartPlaylistKind::NeverPlayed,
        SmartPlaylistKind::LostGems,
    ] {
        assert!(
            store
                .smart_playlist(kind, usize::MAX)
                .expect("smart playlist")
                .is_empty(),
            "{kind:?} reflects the cleared library"
        );
    }

    // Playlists survive with their entries listed — dangling references are
    // valid product behavior validated at read time.
    let playlists = store.load_playlists().expect("playlists load");
    assert_eq!(playlists.len(), 1, "curation survives the wipe");
    assert_eq!(playlists[0].name, "Curation");
    assert_eq!(
        playlists[0].tracks,
        vec![
            TrackId("f:\\cl\\a\\one.mp3".to_string()),
            TrackId("f:\\cl\\gone.mp3".to_string()),
        ],
        "entries stay listed even though their tracks are gone"
    );

    assert_custom_rule_tables_survive(&store);

    // Every setting survives untouched.
    let settings = store.load_settings().expect("settings load");
    assert_eq!(settings.scalars.volume, Some(0.75));
    assert!(settings.scalars.advanced_mode);
    assert!(!settings.scalars.high_contrast);
    assert!(settings.scalars.replaygain_enabled);
    assert_eq!(settings.library_paths, vec![PathBuf::from("f:\\cl")]);
    assert_eq!(
        settings.watch_states.get(&PathBuf::from("f:\\cl")),
        Some(&riff_persistence::store::WatchState::Enabled),
    );
}

#[test]
fn test_clear_library_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    {
        let mut store = seeded_clear_store(&db_path);
        store.clear_library().expect("clear works");
    } // dropped like an app restart.

    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let reopened = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening works");
    assert_eq!(track_count(&reopened), 0);
    assert_eq!(
        reopened.load_playlists().expect("playlists").len(),
        1,
        "curation persists across restarts"
    );
    assert_eq!(
        reopened.load_settings().expect("settings").library_paths,
        vec![PathBuf::from("f:\\cl")],
        "settings persist across restarts"
    );
}

#[test]
fn test_clear_library_is_atomic_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let mut store = seeded_clear_store(&db_path);

    // Simulate a mid-clear failure at the infra seam: an ABORT trigger on
    // the artists table fires after tracks and albums were already deleted
    // inside the transaction.
    store
        .with_connection(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER fail_clear BEFORE DELETE ON artists
                 BEGIN SELECT RAISE(ABORT, 'simulated mid-clear failure'); END;",
            )
        })
        .expect("trigger creation works");

    let outcome = store.clear_library();
    assert!(
        outcome.is_err(),
        "the simulated failure surfaces as an error"
    );

    // The rollback restored every deleted row: nothing partially cleared.
    assert_eq!(
        track_count(&store),
        3,
        "tracks are fully restored after the failed wipe"
    );
    let artists: i64 = store
        .with_connection(|conn| conn.query_row("SELECT COUNT(*) FROM artists", [], |r| r.get(0)))
        .expect("artist count works");
    assert_eq!(artists, 2, "albums/artists are fully restored too");
    assert_eq!(
        store.load_playlists().expect("playlists").len(),
        1,
        "curation is untouched by the failed attempt"
    );

    // Removing the trigger lets the same call succeed cleanly afterwards.
    store
        .with_connection(|conn| conn.execute_batch("DROP TRIGGER fail_clear;"))
        .expect("trigger removal works");
    assert_eq!(store.clear_library().expect("clear works"), 3);
    assert_eq!(track_count(&store), 0);
}

// --- Playlist adapter: session generation bumps only on committed mutations ---

use riff_persistence::store::StoreGeneration;

/// Scratch store plus a clone wired to the session playlist generation
/// handle; `_dir` keeps the database file alive for the test.
struct PlaylistGenerationFixture {
    _dir: tempfile::TempDir,
    shared: riff_infra::store::SqliteStore,
    generation: StoreGeneration,
    store: riff_infra::store::SqliteStore,
}

impl PlaylistGenerationFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("riff.sqlite3");
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let shared = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
            .expect("fresh store must open and migrate");
        let generation = shared.playlist_generation();
        let store = shared.clone();
        Self {
            _dir: dir,
            shared,
            generation,
            store,
        }
    }
}

/// Run one schema batch against the scratch connection (trigger plumbing).
fn exec_batch(shared: &riff_infra::store::SqliteStore, batch: &str) {
    shared
        .with_connection(|conn| conn.execute_batch(batch))
        .expect("schema batch works");
}

#[test]
fn test_playlist_adapter_bumps_generation_on_committed_mutations() {
    let mut fx = PlaylistGenerationFixture::new();
    assert_eq!(fx.generation.current(), 0, "a fresh session starts at zero");

    let t1 = TrackId("a.mp3".to_string());
    let t2 = TrackId("b.mp3".to_string());

    let pid = fx
        .store
        .create_playlist("Focus Mix", &[])
        .expect("create works");
    assert_eq!(fx.generation.current(), 1, "create_playlist commit bumps");

    assert!(fx.store.rename_playlist(&pid, "Workout").unwrap());
    assert_eq!(fx.generation.current(), 2, "rename_playlist commit bumps");

    assert!(fx.store.add_playlist_entry(&pid, &t1).unwrap());
    assert_eq!(
        fx.generation.current(),
        3,
        "add_playlist_entry commit bumps"
    );

    assert!(fx.store.add_playlist_entry(&pid, &t2).unwrap());
    assert_eq!(fx.generation.current(), 4);

    assert!(
        fx.store
            .reorder_playlist_entries(&pid, &[t2.clone(), t1.clone()])
            .unwrap()
    );
    assert_eq!(
        fx.generation.current(),
        5,
        "reorder_playlist_entries commit bumps"
    );

    assert!(fx.store.remove_playlist_entries(&pid, &t1).unwrap());
    assert_eq!(
        fx.generation.current(),
        6,
        "remove_playlist_entries commit bumps"
    );

    assert!(fx.store.delete_playlist(&pid).unwrap());
    assert_eq!(fx.generation.current(), 7, "delete_playlist commit bumps");
}

#[test]
fn test_playlist_adapter_noop_mutations_do_not_bump_generation() {
    let mut fx = PlaylistGenerationFixture::new();
    let t1 = TrackId("a.mp3".to_string());
    let missing = TrackId("gone.mp3".to_string());
    let unknown = PlaylistId("playlist-does-not-exist".to_string());

    // One committed create seeds one entry and sets the baseline every
    // no-op compares against.
    let pid = fx
        .store
        .create_playlist("Focus Mix", std::slice::from_ref(&t1))
        .expect("create works");
    assert_eq!(fx.generation.current(), 1);

    assert!(!fx.store.add_playlist_entry(&pid, &t1).unwrap());
    assert!(!fx.store.remove_playlist_entries(&pid, &missing).unwrap());
    assert!(!fx.store.rename_playlist(&unknown, "New").unwrap());
    assert!(!fx.store.delete_playlist(&unknown).unwrap());
    assert!(!fx.store.add_playlist_entry(&unknown, &t1).unwrap());
    assert!(
        !fx.store
            .reorder_playlist_entries(&unknown, std::slice::from_ref(&t1))
            .unwrap()
    );
    assert_eq!(
        fx.generation.current(),
        1,
        "no-op Ok(false) mutations must not bump the generation"
    );
}
#[test]
fn test_playlist_adapter_failed_mutations_do_not_bump_generation() {
    let mut fx = PlaylistGenerationFixture::new();
    let t1 = TrackId("a.mp3".to_string());
    let t2 = TrackId("b.mp3".to_string());

    // A playlist holding one entry backs the entry-mutation failure cases.
    let pid = fx
        .store
        .create_playlist("Focus Mix", std::slice::from_ref(&t1))
        .expect("create works");
    assert_eq!(fx.generation.current(), 1);

    // Each case aborts exactly one kind of playlist statement mid-transaction
    // (same seam trick as the clear-library atomicity test above).
    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_create BEFORE INSERT ON playlists
         BEGIN SELECT RAISE(ABORT, 'simulated create failure'); END;",
    );
    assert!(
        fx.store.create_playlist("Broken", &[]).is_err(),
        "the simulated failure surfaces as an error"
    );
    assert_eq!(fx.generation.current(), 1, "failed create must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_create;");

    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_rename BEFORE UPDATE ON playlists
         BEGIN SELECT RAISE(ABORT, 'simulated rename failure'); END;",
    );
    assert!(fx.store.rename_playlist(&pid, "Broken").is_err());
    assert_eq!(fx.generation.current(), 1, "failed rename must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_rename;");

    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_delete BEFORE DELETE ON playlists
         BEGIN SELECT RAISE(ABORT, 'simulated delete failure'); END;",
    );
    assert!(fx.store.delete_playlist(&pid).is_err());
    assert_eq!(fx.generation.current(), 1, "failed delete must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_delete;");

    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_add BEFORE INSERT ON playlist_entries
         BEGIN SELECT RAISE(ABORT, 'simulated add failure'); END;",
    );
    assert!(fx.store.add_playlist_entry(&pid, &t2).is_err());
    assert_eq!(fx.generation.current(), 1, "failed add must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_add;");

    // remove and reorder both start by deleting playlist_entries rows.
    exec_batch(
        &fx.shared,
        "CREATE TRIGGER fail_remove BEFORE DELETE ON playlist_entries
         BEGIN SELECT RAISE(ABORT, 'simulated remove failure'); END;",
    );
    assert!(fx.store.remove_playlist_entries(&pid, &t1).is_err());
    assert_eq!(fx.generation.current(), 1, "failed remove must not bump");
    assert!(
        fx.store
            .reorder_playlist_entries(&pid, std::slice::from_ref(&t1))
            .is_err()
    );
    assert_eq!(fx.generation.current(), 1, "failed reorder must not bump");
    exec_batch(&fx.shared, "DROP TRIGGER fail_remove;");

    // Sanity: with the trigger gone the same mutation commits and moves.
    assert!(fx.store.delete_playlist(&pid).unwrap());
    assert_eq!(
        fx.generation.current(),
        2,
        "commits after failures still bump"
    );
}

// --- Store event transactions emit beside the generation bump (issue 04) ----
//
// Moved with the crate from the store adapter's former inline test module;
// assertions unchanged.

mod emit_beside_bump {
    use super::*;

    fn fresh_store() -> (SqliteStore, crossbeam_channel::Receiver<StoreChanged>) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.into_temp_path();
        let (tx, rx) = crossbeam_channel::unbounded();
        let store = SqliteStore::open_and_migrate(&path, tx).unwrap();
        (store, rx)
    }

    #[test]
    fn committed_playlist_mutation_emits_one_notify_and_bumps() {
        let (mut store, rx) = fresh_store();
        let id = TrackId::from_path(std::path::Path::new("track://x/1"));

        let gen_before = store.playlist_generation().current();
        assert_eq!(gen_before, 0);

        let created = store.create_playlist("A", &[id]).unwrap();
        let _ = created;

        let gen_after = store.playlist_generation().current();
        assert_eq!(gen_after, 1);

        // Drain the receiver: exactly one Playlists(1) and nothing else.
        let mut events: Vec<StoreChanged> = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert_eq!(events, vec![StoreChanged::Playlists(1)]);
    }

    #[test]
    fn failing_playlist_mutation_emits_nothing_and_does_not_bump() {
        let (mut store, rx) = fresh_store();
        // Unknown id cannot be renamed — should return false.
        let bad_id = PlaylistId("not-a-playlist".to_string());

        let gen_before = store.playlist_generation().current();

        let renamed = store.rename_playlist(&bad_id, "X").unwrap();
        assert!(!renamed);

        let gen_after = store.playlist_generation().current();
        assert_eq!(gen_before, gen_after);

        // No StoreChanged should have been sent.
        assert!(rx.is_empty());
    }

    #[test]
    fn committed_library_mutation_emits_one_notify_and_bumps() {
        let (mut store, rx) = fresh_store();
        let track = Track::fresh_scan(
            TrackId::from_path(std::path::Path::new("track://x/1")),
            PathBuf::from("track://x/1"),
            TrackMetadata::default(),
            None,
            None,
            None,
            None,
        );

        let gen_before = store.library_generation().current();
        assert_eq!(gen_before, 0);

        store.apply_scan_batch(&[track]).unwrap();

        let gen_after = store.library_generation().current();
        assert_eq!(gen_after, 1);

        let mut events: Vec<StoreChanged> = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        assert_eq!(events, vec![StoreChanged::Library(1)]);
    }
}

// --- Application Store: favorites flag + Favorites smart list (issue 03) ----

#[test]
fn set_track_favorite_round_trips_through_get_track() {
    let (_dir, mut store, _changes_rx) = open_store();

    let track = library_track("m:\\music\\fav\\01.mp3", "Song", None, "Album", None);
    store.apply_scan_batch(&[track]).unwrap();
    let id = TrackId::from_path(std::path::Path::new("m:\\music\\fav\\01.mp3"));

    // Freshly scanned tracks start un-favorited.
    let fresh = store.get_track(&id).unwrap().expect("known track resolves");
    assert!(!fresh.favorite, "a scanned track is not a favorite yet");

    let changed = store.set_track_favorite(&id, true).unwrap();
    assert!(changed, "favoriting a known track commits a change");
    assert!(
        store.get_track(&id).unwrap().expect("still known").favorite,
        "the favorite flag reads back true after setting it"
    );

    let changed = store.set_track_favorite(&id, false).unwrap();
    assert!(changed, "un-favoriting a favorite flips the flag back");
    assert!(
        !store.get_track(&id).unwrap().expect("still known").favorite,
        "the favorite flag reads back false after clearing it"
    );
}

#[test]
fn favorites_smart_list_returns_exactly_the_favorited_tracks() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            library_track("m:\\music\\a.mp3", "A", None, "Album", None),
            library_track("m:\\music\\b.mp3", "B", None, "Album", None),
            library_track("m:\\music\\c.mp3", "C", None, "Album", None),
        ])
        .unwrap();
    let a = TrackId::from_path(std::path::Path::new("m:\\music\\a.mp3"));
    let c = TrackId::from_path(std::path::Path::new("m:\\music\\c.mp3"));

    // An empty favorites list before anything is marked.
    let empty = store
        .smart_playlist(SmartPlaylistKind::Favorites, usize::MAX)
        .unwrap();
    assert!(empty.is_empty(), "nothing is favorited yet");

    store.set_track_favorite(&c, true).unwrap();
    store.set_track_favorite(&a, true).unwrap();

    let favorites = store
        .smart_playlist(SmartPlaylistKind::Favorites, usize::MAX)
        .unwrap();
    assert_eq!(
        favorites
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["m:\\music\\a.mp3", "m:\\music\\c.mp3"],
        "exactly the marked tracks, in canonical path order"
    );
}

#[test]
fn favorite_flag_survives_store_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let id = TrackId::from_path(std::path::Path::new("m:\\music\\fav\\01.mp3"));

    {
        let (changes_tx, _changes_rx) =
            crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
        let mut store =
            riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        store
            .apply_scan_batch(&[library_track(
                "m:\\music\\fav\\01.mp3",
                "Song",
                None,
                "Album",
                None,
            )])
            .unwrap();
        store.set_track_favorite(&id, true).unwrap();
    }

    // A whole new store handle over the same file — the restart path.
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let store = riff_infra::store::SqliteStore::open_and_migrate(&db_path, changes_tx)
        .expect("reopening the store must succeed");
    assert!(
        store.get_track(&id).unwrap().expect("still known").favorite,
        "the favorite flag reads back true after a restart"
    );
}

#[test]
fn rescanning_and_tag_refreshing_a_favorited_track_preserve_the_flag() {
    let (_dir, mut store, _changes_rx) = open_store();

    let path = "m:\\music\\fav\\01.mp3";
    let id = TrackId::from_path(std::path::Path::new(path));
    store
        .apply_scan_batch(&[library_track(path, "Song", None, "Album", None)])
        .unwrap();
    store.set_track_favorite(&id, true).unwrap();

    // A rescan re-upserts the track from scan-side data (favorite: false
    // there) — the stored flag must survive untouched, like play history.
    store
        .apply_scan_batch(&[library_track(path, "Song", None, "Album", None)])
        .unwrap();
    assert!(
        store.get_track(&id).unwrap().expect("still known").favorite,
        "a rescan must not clear the favorite flag"
    );

    // The tag-edit refresh shares the upsert; it must preserve the flag too.
    store
        .apply_tag_refresh(&library_track(path, "Renamed", None, "Album", None))
        .unwrap();
    assert!(
        store.get_track(&id).unwrap().expect("still known").favorite,
        "a tag edit must not clear the favorite flag"
    );
}

#[test]
fn favorites_query_orders_by_path_regardless_of_marking_order() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            library_track("m:\\music\\zeta.mp3", "Z", None, "Album", None),
            library_track("m:\\music\\mid.mp3", "M", None, "Album", None),
            library_track("m:\\music\\alpha.mp3", "A", None, "Album", None),
        ])
        .unwrap();
    let zeta = TrackId::from_path(std::path::Path::new("m:\\music\\zeta.mp3"));
    let mid = TrackId::from_path(std::path::Path::new("m:\\music\\mid.mp3"));
    let alpha = TrackId::from_path(std::path::Path::new("m:\\music\\alpha.mp3"));

    // Mark in an order deliberately opposite to the canonical path order.
    store.set_track_favorite(&zeta, true).unwrap();
    store.set_track_favorite(&alpha, true).unwrap();
    store.set_track_favorite(&mid, true).unwrap();

    let favorites = store
        .smart_playlist(SmartPlaylistKind::Favorites, usize::MAX)
        .unwrap();
    assert_eq!(
        favorites
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        vec![
            "m:\\music\\alpha.mp3",
            "m:\\music\\mid.mp3",
            "m:\\music\\zeta.mp3"
        ],
        "favorites list in path-ascending order, not marking order"
    );

    // The limit truncates the already-ordered list.
    let bounded = store
        .smart_playlist(SmartPlaylistKind::Favorites, 2)
        .unwrap();
    assert_eq!(bounded.len(), 2);
    assert_eq!(bounded[0].id.0, "m:\\music\\alpha.mp3");
}

#[test]
fn removing_and_clearing_the_library_drop_their_favorites() {
    let (_dir, mut store, _changes_rx) = open_store();

    store
        .apply_scan_batch(&[
            library_track("m:\\lib\\keep\\01.mp3", "Keep", None, "Album", None),
            library_track("m:\\lib\\gone\\01.mp3", "Gone", None, "Album", None),
        ])
        .unwrap();
    let keep = TrackId::from_path(std::path::Path::new("m:\\lib\\keep\\01.mp3"));
    let gone = TrackId::from_path(std::path::Path::new("m:\\lib\\gone\\01.mp3"));
    store.set_track_favorite(&keep, true).unwrap();
    store.set_track_favorite(&gone, true).unwrap();

    store
        .remove_library_path(std::path::Path::new("m:\\lib\\gone"))
        .unwrap();

    let favorites = store
        .smart_playlist(SmartPlaylistKind::Favorites, usize::MAX)
        .unwrap();
    assert_eq!(
        favorites
            .iter()
            .map(|t| t.id.0.as_str())
            .collect::<Vec<_>>(),
        vec!["m:\\lib\\keep\\01.mp3"],
        "the removed root's favorite is gone; the other survives"
    );

    // The maintenance wipe leaves no favorites at all.
    store.clear_library().unwrap();
    let favorites = store
        .smart_playlist(SmartPlaylistKind::Favorites, usize::MAX)
        .unwrap();
    assert!(
        favorites.is_empty(),
        "clearing the library clears favorites"
    );
}

#[test]
fn committing_a_favorite_toggle_bumps_library_generation_and_emits_once() {
    let (_dir, _, mut store, changes_rx) = open_store_with_path();

    store
        .apply_scan_batch(&[library_track(
            "m:\\music\\fav\\01.mp3",
            "Song",
            None,
            "Album",
            None,
        )])
        .unwrap();
    let _ = changes_rx.try_recv(); // drain the scan-batch notification
    let id = TrackId::from_path(std::path::Path::new("m:\\music\\fav\\01.mp3"));

    let gen_before = store.library_generation().current();
    store.set_track_favorite(&id, true).unwrap();
    assert_eq!(
        store.library_generation().current(),
        gen_before + 1,
        "a committed favorite toggle bumps the Library generation"
    );
    let events: Vec<StoreChanged> = {
        let mut events = Vec::new();
        while let Ok(e) = changes_rx.try_recv() {
            events.push(e);
        }
        events
    };
    assert_eq!(
        events,
        vec![StoreChanged::Library(gen_before + 1)],
        "exactly one Library notification rides beside the bump"
    );

    // A redundant set and an unknown id change nothing and emit nothing.
    store.set_track_favorite(&id, true).unwrap();
    let unknown = TrackId::from_path(std::path::Path::new("m:\\music\\missing.mp3"));
    let changed = store.set_track_favorite(&unknown, true).unwrap();
    assert!(!changed, "an unknown track reports no change");
    assert_eq!(
        store.library_generation().current(),
        gen_before + 1,
        "no-ops must not bump the generation"
    );
    assert!(changes_rx.is_empty(), "no-ops must not emit");
}

// --- Listing Page: one connection acquisition (listing-page-read 02) --------

/// A Listing Page is one fact read at one generation: the store takes the
/// connection once for both halves, so a writer on another thread can only
/// land wholly before or wholly after the read. A read that took the
/// connection twice would let a commit land in between and report a total
/// counting a track its window never saw (or the reverse).
///
/// The invariant is therefore that the window is exactly as full as the total
/// permits — `rows == min(total, window)` — not that `total == rows`. The
/// strong form is what the capped one says while the collection still fits the
/// window (which is where a torn read shows up), but the writer is only
/// released once the reader has taken its minimum frames, so under suite load
/// it overshoots the window and the strong form fails on a read that mixed
/// nothing.
#[test]
fn a_page_read_never_mixes_generations_with_a_concurrent_writer() {
    // Both threads must put in their minimum, so a fast reader cannot let the
    // writer retire early and a slow one cannot starve the interleaving: the
    // race window is guaranteed to be wide in both directions. The reader's
    // own read count is never asserted, so no wall-clock timing can fail this
    // test -- which is what a fixed read-count floor did under suite load.
    const MIN_WRITES: usize = 400;
    const MIN_FRAMES: usize = 400;
    const WINDOW: usize = MIN_WRITES * 4;

    let (_dir, _, store, _changes_rx) = open_store_with_path();

    let frames = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let writing = Arc::new(AtomicBool::new(true));
    let writing_in_writer = Arc::clone(&writing);
    let frames_in_writer = Arc::clone(&frames);
    let mut writer = store.clone();
    let writer = std::thread::spawn(move || {
        let mut written = 0usize;
        loop {
            writer
                .apply_scan_batch(&[library_track(
                    &format!("m:/music/{written:04}.mp3"),
                    "Concurrent",
                    Some("Ada"),
                    "One",
                    Some("Ada"),
                )])
                .expect("concurrent scan batch must commit");
            written += 1;
            if written >= MIN_WRITES
                && frames_in_writer.load(std::sync::atomic::Ordering::Acquire) >= MIN_FRAMES
            {
                break;
            }
        }
        writing_in_writer.store(false, std::sync::atomic::Ordering::Release);
        written
    });

    let reader = store.clone();
    let mut reads = 0usize;
    while writing.load(std::sync::atomic::Ordering::Acquire) {
        let page = reader
            .tracks_page(TrackListOrder::default(), 0, WINDOW)
            .expect("page read must succeed");
        assert_eq!(
            page.rows().len(),
            page.total().min(WINDOW),
            "read {reads}: a page read taken while writes land must deliver exactly as \
             many rows as its total and its window allow"
        );
        reads += 1;
        frames.store(reads, std::sync::atomic::Ordering::Release);
    }
    let written = writer.join().unwrap();

    assert!(
        written >= MIN_WRITES,
        "the writer must have kept committing across the read window, made {written} commits"
    );

    // Settled: with the writer joined its count is known, so the window is
    // asked wide enough that the capped form above is the strong one.
    let page = reader
        .tracks_page(TrackListOrder::default(), 0, written + 1)
        .unwrap();
    assert_eq!(page.total(), written, "the writer's batches all committed");
    assert_eq!(page.rows().len(), written, "and every one is listed");
}

// --- Application Store: the one folded mutation prologue (deepen-iter1 01) --
//
// Every mutating store method runs the same prologue: take the connection
// lock, run ONE immediate durable transaction, wrap a failure in the
// operation's own context, and bump the session generation the mutation
// names exactly when it committed. These tests lock that rule site by site
// so folding the 17 hand-written prologues into one helper cannot drift from
// it: a committing site is durable AND bumps with exactly one notice beside
// the bump, while the writes that deliberately do NOT bump
// (`stamp_metadata_version` and the Settings setters) stay durable with both
// generations unmoved.

/// Every notice the change channel has queued so far, in arrival order.
fn drain_changes(rx: &crossbeam_channel::Receiver<StoreChanged>) -> Vec<StoreChanged> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

/// The Library mutations: each one is durable across a reopen and bumps the
/// session Library generation exactly once, with that one notice beside it.
#[test]
fn library_mutations_are_durable_and_bump_the_library_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let track = library_track(
        "m:\\music\\folded\\01.mp3",
        "Song",
        Some("Artist"),
        "Album",
        Some("Album Artist"),
    );
    let summary = FullScanSummary {
        at: SystemTime::now(),
        files: 1,
        errors: 0,
    };

    {
        let (changes_tx, changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
        let mut store = SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let library_gen = store.library_generation();
        let playlist_gen = store.playlist_generation();
        assert_eq!(library_gen.current(), 0, "a fresh session starts at zero");
        assert_eq!(playlist_gen.current(), 0);

        // Five committing mutations, each one a bump of the Library
        // generation and nothing else.
        assert_eq!(
            store
                .apply_scan_batch(std::slice::from_ref(&track))
                .unwrap(),
            1
        );
        assert_eq!(library_gen.current(), 1, "apply_scan_batch commit bumps");
        assert!(store.record_track_played(&track.id, summary.at).unwrap());
        assert_eq!(library_gen.current(), 2, "record_track_played commit bumps");
        assert!(store.set_track_favorite(&track.id, true).unwrap());
        assert_eq!(library_gen.current(), 3, "set_track_favorite commit bumps");
        store.apply_tag_refresh(&track).unwrap();
        assert_eq!(library_gen.current(), 4, "apply_tag_refresh commit bumps");
        store.record_full_scan_completed(summary).unwrap();
        assert_eq!(
            library_gen.current(),
            5,
            "record_full_scan_completed commit bumps"
        );

        assert_eq!(
            drain_changes(&changes_rx),
            vec![
                StoreChanged::Library(1),
                StoreChanged::Library(2),
                StoreChanged::Library(3),
                StoreChanged::Library(4),
                StoreChanged::Library(5),
            ],
            "each bump carries exactly one Library notice, in commit order"
        );
        assert_eq!(
            playlist_gen.current(),
            0,
            "no Library mutation moves the playlist generation"
        );
    }

    // The "restart": every one of those mutations committed, so the reopened
    // store reads them all back.
    let (reopen_tx, _reopen_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let reopened = SqliteStore::open_and_migrate(&db_path, reopen_tx).unwrap();
    assert_eq!(
        reopened.all_track_ids().unwrap(),
        vec![track.id.clone()],
        "the scan batch committed"
    );
    let stored = reopened
        .get_track(&track.id)
        .unwrap()
        .expect("the track resolves");
    assert_eq!(stored.play_count, 1, "the recorded play committed");
    assert!(stored.favorite, "the favorite toggle committed");
    assert_eq!(
        reopened.last_full_scan().unwrap(),
        Some(summary),
        "the recorded scan summary committed"
    );
}

/// The playlist mutations: each one is durable across a reopen and bumps the
/// session playlist generation exactly once, with that one notice beside it.
#[test]
fn playlist_mutations_are_durable_and_bump_the_playlist_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    {
        let (changes_tx, changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
        let mut store = SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let library_gen = store.library_generation();
        let playlist_gen = store.playlist_generation();
        assert_eq!(playlist_gen.current(), 0, "a fresh session starts at zero");

        // Six committing mutations, each one a bump of the playlist
        // generation and nothing else.
        let first = TrackId("first.mp3".to_string());
        let second = TrackId("second.mp3".to_string());
        let third = TrackId("third.mp3".to_string());
        let playlist_id = store
            .create_playlist("Folded", &[first.clone(), second.clone()])
            .unwrap();
        assert_eq!(playlist_gen.current(), 1, "create_playlist commit bumps");
        assert!(store.rename_playlist(&playlist_id, "Folded Again").unwrap());
        assert_eq!(playlist_gen.current(), 2, "rename_playlist commit bumps");
        assert!(store.add_playlist_entry(&playlist_id, &third).unwrap());
        assert_eq!(playlist_gen.current(), 3, "add_playlist_entry commit bumps");
        assert!(
            store
                .reorder_playlist_entries(
                    &playlist_id,
                    &[third.clone(), first.clone(), second.clone()]
                )
                .unwrap()
        );
        assert_eq!(
            playlist_gen.current(),
            4,
            "reorder_playlist_entries commit bumps"
        );
        assert!(store.remove_playlist_entries(&playlist_id, &first).unwrap());
        assert_eq!(
            playlist_gen.current(),
            5,
            "remove_playlist_entries commit bumps"
        );
        assert!(store.delete_playlist(&playlist_id).unwrap());
        assert_eq!(playlist_gen.current(), 6, "delete_playlist commit bumps");

        assert_eq!(
            drain_changes(&changes_rx),
            vec![
                StoreChanged::Playlists(1),
                StoreChanged::Playlists(2),
                StoreChanged::Playlists(3),
                StoreChanged::Playlists(4),
                StoreChanged::Playlists(5),
                StoreChanged::Playlists(6),
            ],
            "each bump carries exactly one Playlists notice, in commit order"
        );
        assert_eq!(
            library_gen.current(),
            0,
            "no playlist mutation moves the Library generation"
        );
    }

    // The "restart": the whole sequence committed, and the delete that ended
    // it is what the reopened store sees — a playlist that was created,
    // renamed, reordered and deleted is gone, so the delete committed.
    let (reopen_tx, _reopen_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let reopened = SqliteStore::open_and_migrate(&db_path, reopen_tx).unwrap();
    assert!(
        reopened.load_playlists().unwrap().is_empty(),
        "the deleted playlist committed"
    );
}

/// The two destructive Library sweeps: each commits its whole sweep and
/// bumps the Library generation once for it.
#[test]
fn clear_library_and_remove_library_path_commit_and_bump_once_each() {
    let (_dir, mut store, changes_rx) = open_store();
    let library_gen = store.library_generation();
    let root = std::path::Path::new("m:\\music\\sweep");
    let kept = library_track("m:\\music\\keep\\01.mp3", "Keep", None, "Keep", None);
    let gone = library_track("m:\\music\\sweep\\01.mp3", "Gone", None, "Gone", None);

    store
        .apply_scan_batch(&[kept.clone(), gone.clone()])
        .unwrap();
    assert_eq!(library_gen.current(), 1, "the seeding batch bumps");
    assert_eq!(
        drain_changes(&changes_rx),
        vec![StoreChanged::Library(1)],
        "the seeding batch emits one notice"
    );

    assert_eq!(
        store.clear_library().unwrap(),
        2,
        "clear removes every track"
    );
    assert_eq!(library_gen.current(), 2, "clear_library commit bumps");
    assert!(
        store.all_track_ids().unwrap().is_empty(),
        "the clear committed"
    );

    store
        .apply_scan_batch(&[kept.clone(), gone.clone()])
        .unwrap();
    assert_eq!(library_gen.current(), 3, "the re-seeded batch bumps");

    assert_eq!(
        store.remove_library_path(root).unwrap(),
        1,
        "only the root's own tracks go"
    );
    assert_eq!(library_gen.current(), 4, "remove_library_path commit bumps");
    assert_eq!(
        store.all_track_ids().unwrap(),
        vec![kept.id.clone()],
        "the remove committed"
    );

    assert_eq!(
        drain_changes(&changes_rx),
        vec![
            StoreChanged::Library(2),
            StoreChanged::Library(3),
            StoreChanged::Library(4),
        ],
        "each sweep's commit emits exactly one notice beside its bump"
    );
}

/// `stamp_metadata_version` is the one durable mutation that deliberately
/// does NOT bump: the version describes how the collection was read, so no
/// Session Projection has to refetch for it. The write is durable, both
/// generations stay unmoved, no notice rides out — and a later commit still
/// bumps, so the exception is per call rather than sticky.
#[test]
fn stamping_the_metadata_version_commits_without_bumping_either_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    {
        let (changes_tx, changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
        let mut store = SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let library_gen = store.library_generation();
        let playlist_gen = store.playlist_generation();

        store.stamp_metadata_version(42).unwrap();

        assert_eq!(
            library_gen.current(),
            0,
            "the stamp does not bump the Library generation"
        );
        assert_eq!(
            playlist_gen.current(),
            0,
            "the stamp does not bump the playlist generation"
        );
        assert!(changes_rx.is_empty(), "the stamp emits no notice");

        // The exception belongs to that one call, not to the store: the next
        // committed playlist mutation still bumps.
        store.create_playlist("After the Stamp", &[]).unwrap();
        assert_eq!(playlist_gen.current(), 1, "a later commit still bumps");
        assert_eq!(
            drain_changes(&changes_rx),
            vec![StoreChanged::Playlists(1)],
            "only the commit's notice rides out"
        );
    }

    // The stamp itself is durable: the reopened store reads the version back
    // from a fresh session whose counters still start at zero.
    let (reopen_tx, _reopen_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let reopened = SqliteStore::open_and_migrate(&db_path, reopen_tx).unwrap();
    assert_eq!(
        reopened.metadata_version().unwrap(),
        42,
        "the stamp is durable"
    );
    assert_eq!(
        reopened.playlist_generation().current(),
        0,
        "the stamping session's counter is unmoved, and a new one starts at zero"
    );
}

/// The three Settings setters commit like every other mutation but bump
/// neither generation: Settings reads are not generation-keyed projections.
#[test]
fn settings_setters_commit_without_bumping_either_generation() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");

    {
        let (changes_tx, changes_rx) = crossbeam_channel::unbounded::<StoreChanged>();
        let mut store = SqliteStore::open_and_migrate(&db_path, changes_tx).unwrap();
        let library_gen = store.library_generation();
        let playlist_gen = store.playlist_generation();

        store
            .save_scalars(&ScalarSettings {
                volume: Some(0.5),
                ..ScalarSettings::default()
            })
            .unwrap();
        store
            .save_library_paths(&[PathBuf::from("m:\\music")])
            .unwrap();
        let mut watch_states = HashMap::new();
        watch_states.insert(PathBuf::from("m:\\music"), WatchState::Enabled);
        store.save_watch_states(&watch_states).unwrap();

        assert_eq!(
            library_gen.current(),
            0,
            "settings writes do not bump the Library generation"
        );
        assert_eq!(
            playlist_gen.current(),
            0,
            "settings writes do not bump the playlist generation"
        );
        assert!(changes_rx.is_empty(), "settings writes emit no notice");
    }

    let (reopen_tx, _reopen_rx) = crossbeam_channel::unbounded::<StoreChanged>();
    let reopened = SqliteStore::open_and_migrate(&db_path, reopen_tx).unwrap();
    let settings = reopened.load_settings().unwrap();
    assert_eq!(
        settings.scalars.volume,
        Some(0.5),
        "the scalar block committed"
    );
    assert_eq!(
        settings.library_paths,
        vec![PathBuf::from("m:\\music")],
        "the library paths committed"
    );
    assert_eq!(
        settings.watch_states.get(&PathBuf::from("m:\\music")),
        Some(&WatchState::Enabled),
        "the watch states committed"
    );
}

/// A mutation that aborts mid-transaction surfaces its own operation's
/// context: the folded prologue wraps the failure through the same
/// per-operation error wrapping the sites spelled out by hand.
#[test]
fn a_failed_mutation_reports_its_own_operation_context() {
    let (_dir, mut store, changes_rx) = open_store();
    let id = store.create_playlist("Known", &[]).unwrap();
    let library_gen = store.library_generation();
    let playlist_gen = store.playlist_generation();
    assert_eq!(playlist_gen.current(), 1, "the seeding commit bumps");
    assert_eq!(
        drain_changes(&changes_rx),
        vec![StoreChanged::Playlists(1)],
        "the seeding commit emits its one notice"
    );

    exec_batch(
        &store,
        "CREATE TRIGGER fail_rename BEFORE UPDATE ON playlists
         BEGIN SELECT RAISE(ABORT, 'simulated rename failure'); END;",
    );
    let err = store.rename_playlist(&id, "Broken").unwrap_err();
    exec_batch(&store, "DROP TRIGGER fail_rename;");

    assert!(
        err.to_string().contains("failed to rename playlist"),
        "the failure carries the operation's own context: {err}"
    );
    assert_eq!(
        playlist_gen.current(),
        1,
        "a failed mutation does not bump the generation it names"
    );
    assert_eq!(
        library_gen.current(),
        0,
        "and it leaves the other generation alone"
    );
    assert!(changes_rx.is_empty(), "a failed mutation emits no notice");
}
// --- Application Store: Rule trees as SQL (ticket 09) ----------------------

// The Custom Smart Playlist oracle: a seeded Library, the store's own answer
// through the compiled predicate, and a Rust-only walk of the same tree that
// must agree with it. `rules_reference`'s module doc states the one rule that
// makes this arrangement worth anything — read it before unifying anything.

use riff_persistence::rule::{Comparison, MatchableField};

/// A row of the Rule corpus: a stored path plus every Matchable Field the path
/// and its tags can carry. Every field is `None` / absent unless a test shape
/// needs a value, so one row names only what it pins.
#[derive(Default)]
struct RulesRow {
    /// The file name under `r:\rules\`; its extension is the `lossless` fact.
    name: &'static str,
    title: Option<&'static str>,
    artist: Option<&'static str>,
    album: Option<&'static str>,
    album_artist: Option<&'static str>,
    composer: Option<&'static str>,
    comment: Option<&'static str>,
    genre: Option<&'static str>,
    year: Option<u32>,
    /// Whole milliseconds — the unit a duration Rule holds. The corpus stores
    /// no fractional millisecond, which is what makes the 1 ms = 1e6 ns
    /// translation exact on both sides.
    duration_millis: Option<u64>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    play_count: u32,
    /// The favorite flag, seeded through the mutation port rather than the
    /// scan — it is a user fact, not tag data.
    favorite: bool,
    /// A local civil `before` / `after` instant is what a history Rule holds,
    /// so the corpus names its history dates in the same domain.
    last_played: Option<CivilDateTime>,
    date_added: Option<CivilDateTime>,
    /// Days back from the read, for the `in_last` window. Kept apart from the
    /// civil instants above because a window moves with the clock and a stored
    /// boundary does not.
    last_played_days_ago: Option<u64>,
    date_added_days_ago: Option<u64>,
}

/// The civil instant a corpus row names, spelled as the parts a Rule stores.
pub(crate) const fn civil(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
) -> CivilDateTime {
    CivilDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

/// The Rule corpus. One comment per row naming what it pins, and every shape
/// the spec's fixture minimum asks for. The table is data rather than logic,
/// so it is one table and deliberately not split.
#[allow(clippy::too_many_lines, reason = "one corpus row per pinned shape")]
fn rules_fixtures() -> Vec<RulesRow> {
    vec![
        // The `;`-separated genre tag, and the local-midnight pair's earlier
        // half: last played 23:30 on the 1st, added 09:00 on the 1st.
        RulesRow {
            name: "one.flac",
            title: Some("Alpha"),
            artist: Some("Known Artist"),
            album: Some("First"),
            album_artist: Some("AA"),
            composer: Some("Bach"),
            comment: Some("liner notes"),
            genre: Some("Art Rock;Progressive Rock"),
            year: Some(1975),
            duration_millis: Some(300_000),
            sample_rate: Some(44_100),
            channels: Some(2),
            play_count: 3,
            favorite: true,
            last_played: Some(civil(2024, 3, 1, 23, 30, 0)),
            date_added: Some(civil(2024, 6, 1, 9, 0, 0)),
            ..RulesRow::default()
        },
        // The same genre tag with a TAB padding the second segment: the genres
        // view trims it and shows "Progressive Rock"; a Rule does not, so it
        // matches nothing. The documented disagreement, and the only `.wav`.
        // Also the later half of both local-midnight pairs.
        RulesRow {
            name: "two.wav",
            title: Some("Beta"),
            artist: Some("Known Artist"),
            album: Some("First"),
            album_artist: Some("AA"),
            composer: Some("Vivaldi"),
            comment: Some("liner notes"),
            genre: Some("Art Rock;\tProgressive Rock"),
            year: Some(1990),
            // Exactly one hour: the round-millisecond threshold this row and
            // the next one split. `> 3_600_000` must leave this out.
            duration_millis: Some(3_600_000),
            sample_rate: Some(48_000),
            channels: Some(5),
            last_played: Some(civil(2024, 3, 2, 0, 30, 0)),
            date_added: Some(civil(2024, 6, 2, 9, 0, 0)),
            ..RulesRow::default()
        },
        // One millisecond over an hour — the other half of the round threshold.
        // Its genre is the EMPTY STRING, which `is missing` must answer the
        // same way as the NULL genre on the next row.
        RulesRow {
            name: "three.mp3",
            title: Some("Gamma"),
            artist: Some("Known Artist"),
            album: Some("Second"),
            album_artist: Some("BB"),
            genre: Some(""),
            year: Some(2001),
            duration_millis: Some(3_600_001),
            sample_rate: Some(44_100),
            channels: Some(2),
            play_count: 7,
            favorite: true,
            date_added_days_ago: Some(2),
            ..RulesRow::default()
        },
        // `.m4a` — pinned NOT lossless (an ALAC in an m4a container reports as
        // lossy), a NULL genre beside the empty-string one above, an absent
        // artist tag, no duration, and a date_added with no last_played.
        RulesRow {
            name: "four.m4a",
            title: Some("Delta"),
            album: Some("Second"),
            album_artist: Some("BB"),
            composer: Some("Bach"),
            year: Some(1975),
            sample_rate: Some(96_000),
            channels: Some(1),
            last_played: Some(civil(2024, 1, 5, 12, 0, 0)),
            date_added_days_ago: Some(60),
            ..RulesRow::default()
        },
        // An UPPERCASE `.FLAC` — lossless is a case-insensitive suffix match on
        // the stored path. Carries the only literally-tagged `Unknown Artist`
        // beside the absent-artist rows above, and a `date_added` far enough
        // back to fall outside every window the corpus asks about.
        RulesRow {
            name: "five.FLAC",
            title: Some("Echo"),
            artist: Some("Unknown Artist"),
            album: Some("Third"),
            album_artist: Some("CC"),
            comment: Some("notes"),
            genre: Some("Ambient"),
            year: Some(1988),
            duration_millis: Some(120_000),
            sample_rate: Some(44_100),
            channels: Some(2),
            play_count: 1,
            date_added_days_ago: Some(400),
            ..RulesRow::default()
        },
        // A file with NO extension at all: not lossless, and absent year,
        // duration, sample rate, channels and `date_added` — the row that
        // answers `is missing` for every numeric and date field.
        RulesRow {
            name: "six",
            title: Some("Foxtrot"),
            artist: Some("Known Artist"),
            album: Some("Third"),
            album_artist: Some("CC"),
            comment: Some("notes"),
            genre: Some("Ambient"),
            ..RulesRow::default()
        },
        // Absent artist tag again, with a title, so `artist is not "Unknown
        // Artist"` has to include it — the promise a bare `NOT` in SQL breaks.
        RulesRow {
            name: "seven.mp3",
            title: Some("Golf"),
            album: Some("Fourth"),
            album_artist: Some("AA"),
            genre: Some("Electronic"),
            year: Some(1975),
            duration_millis: Some(500_000),
            sample_rate: Some(44_100),
            channels: Some(2),
            play_count: 3,
            favorite: true,
            date_added_days_ago: Some(20),
            ..RulesRow::default()
        },
        // The literal `Unknown Artist` tag, pickable; the row above only looks
        // like it because of the display fallback, and a Rule reads the tag.
        RulesRow {
            name: "eight.mp3",
            title: Some("Hotel"),
            artist: Some("Unknown Artist"),
            album: Some("Fourth"),
            album_artist: Some("AA"),
            genre: Some("Electronic"),
            year: Some(2010),
            duration_millis: Some(500_000),
            sample_rate: Some(44_100),
            channels: Some(2),
            date_added_days_ago: Some(20),
            ..RulesRow::default()
        },
        // A non-Latin tag in UPPERCASE, and the same tag in lowercase two rows
        // down: SQLite's `lower()` folds ASCII only (ADR 0020), so `genre is
        // "Électronique"` picks the lowercase row and not this one. The
        // limitation, pinned.
        RulesRow {
            name: "nine.mp3",
            title: Some("India"),
            artist: Some("Known Artist"),
            album: Some("Fifth"),
            album_artist: Some("DD"),
            genre: Some("ÉLECTRONIQUE"),
            year: Some(1999),
            sample_rate: Some(22_050),
            channels: Some(1),
            play_count: 2,
            date_added_days_ago: Some(20),
            ..RulesRow::default()
        },
        // The lowercase half of the non-Latin pair.
        RulesRow {
            name: "ten.mp3",
            title: Some("Juliet"),
            artist: Some("Known Artist"),
            album: Some("Fifth"),
            album_artist: Some("DD"),
            genre: Some("électronique"),
            year: Some(1999),
            sample_rate: Some(22_050),
            channels: Some(1),
            date_added_days_ago: Some(20),
            ..RulesRow::default()
        },
        // Every tag absent, no history, no plays, not a favorite: the row that
        // makes "is not" and "is missing" mean something on all sixteen fields.
        RulesRow {
            name: "eleven.mp3",
            ..RulesRow::default()
        },
        // A title and a comment holding a quote, a `%` and a `_`: needles for
        // those must match literally, with no wildcard and no escape in sight.
        RulesRow {
            name: "quote.mp3",
            title: Some("He said \"yes\" 100%"),
            artist: Some("Known Artist"),
            album: Some("Sixth"),
            album_artist: Some("EE"),
            comment: Some("50%_off"),
            genre: Some("Trance"),
            year: Some(2020),
            duration_millis: Some(61_000),
            sample_rate: Some(44_100),
            channels: Some(2),
            play_count: 3,
            date_added_days_ago: Some(3),
            ..RulesRow::default()
        },
        // The needle trap's other half: `%_` in a title, which a `LIKE` would
        // read as "anything, then any one character".
        RulesRow {
            name: "oddball.mp3",
            title: Some("100%_done"),
            artist: Some("Known Artist"),
            album: Some("Sixth"),
            album_artist: Some("EE"),
            genre: Some("Trance"),
            year: Some(2020),
            duration_millis: Some(61_000),
            sample_rate: Some(44_100),
            channels: Some(2),
            favorite: true,
            date_added_days_ago: Some(3),
            ..RulesRow::default()
        },
        // Played and added two days ago: inside every short window, outside
        // none of the boundaries.
        RulesRow {
            name: "recent.mp3",
            title: Some("November"),
            artist: Some("Known Artist"),
            album: Some("Seventh"),
            album_artist: Some("FF"),
            genre: Some("Ambient"),
            year: Some(2024),
            duration_millis: Some(180_000),
            sample_rate: Some(44_100),
            channels: Some(2),
            play_count: 1,
            last_played_days_ago: Some(2),
            date_added_days_ago: Some(1),
            ..RulesRow::default()
        },
        // Played 400 days ago and added 500 days ago: the long-window tail.
        RulesRow {
            name: "aged.mp3",
            title: Some("Oscar"),
            artist: Some("Known Artist"),
            album: Some("Seventh"),
            album_artist: Some("FF"),
            genre: Some("Ambient"),
            year: Some(2019),
            duration_millis: Some(180_000),
            sample_rate: Some(44_100),
            channels: Some(1),
            play_count: 12,
            last_played_days_ago: Some(400),
            date_added_days_ago: Some(500),
            ..RulesRow::default()
        },
    ]
}

/// The store's signed UTC epoch-nanosecond encoding, restated for seeding (the
/// adapter's own copy is crate-private, and the corpus names its history in
/// `SystemTime`s like every other fixture does).
fn signed_nanos(time: SystemTime) -> i64 {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(forward) => i64::try_from(forward.as_nanos()).unwrap_or(i64::MAX),
        Err(back) => -i64::try_from(back.duration().as_nanos()).unwrap_or(i64::MAX),
    }
}

/// Days back from `now`, at the same hour the corpus names, so a window's
/// margin from its boundary is whole days rather than a few seconds.
fn days_ago(now: SystemTime, days: u64) -> SystemTime {
    now - Duration::from_secs(days * 86_400)
}

/// The machine's own UTC offset in seconds, read from the same clock the store
/// stamps history with. `strftime('%s', …, 'localtime')` re-reads a local civil
/// value as if it were UTC, so the difference *is* the offset; rounding to the
/// whole minute keeps a second of slop out of the arithmetic.
///
/// This is a measurement of the environment, not of the compiler: the two sides
/// convert a zone-less Rule value into an instant by different mechanisms (the
/// store renders the column as local civil text inside SQL, the reference does
/// integer arithmetic here) and share only this number.
fn local_offset_seconds(store: &SqliteStore) -> i64 {
    let raw = store
        .with_connection(|conn| {
            conn.query_row(
                "SELECT CAST(strftime('%s', 'now', 'localtime') AS INTEGER) \
                    - CAST(strftime('%s', 'now', 'utc') AS INTEGER)",
                [],
                |row| row.get::<_, i64>(0),
            )
        })
        .expect("reading the machine's zone must work");
    raw.div_euclid(60) * 60
}

/// Days since 1970-01-01 for a proleptic-Gregorian civil date — the civil↔epoch
/// arithmetic the store does inside `strftime`, restated in Rust so the
/// reference can name the same instant without SQLite's help.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let year_of_era = y - era * 400;
    let month_shift = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shift + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The instant a zone-less civil value names *on this machine*: the listener's
/// own clock is the whole of its meaning, which is why no zone is stored.
fn local_instant(civil: &CivilDateTime, offset_seconds: i64) -> SystemTime {
    let seconds = days_from_civil(
        i64::from(civil.year),
        i64::from(civil.month),
        i64::from(civil.day),
    ) * 86_400
        + i64::from(civil.hour) * 3_600
        + i64::from(civil.minute) * 60
        + i64::from(civil.second)
        - offset_seconds;
    let offset = Duration::from_secs(seconds.unsigned_abs());
    if seconds >= 0 {
        SystemTime::UNIX_EPOCH + offset
    } else {
        SystemTime::UNIX_EPOCH - offset
    }
}

/// The corpus as stored `Track`s, with every history instant resolved against
/// the machine's own zone (the same resolution a `before` Rule is read at).
fn rules_tracks(rows: &[RulesRow], offset_seconds: i64, now: SystemTime) -> Vec<Track> {
    rows.iter()
        .map(|row| {
            let path = format!("r:\\rules\\{}", row.name);
            let last_played = row
                .last_played
                .as_ref()
                .map(|instant| local_instant(instant, offset_seconds))
                .or_else(|| row.last_played_days_ago.map(|days| days_ago(now, days)));
            let date_added = row
                .date_added
                .as_ref()
                .map(|instant| local_instant(instant, offset_seconds))
                .or_else(|| row.date_added_days_ago.map(|days| days_ago(now, days)));
            Track {
                id: TrackId::from_path(std::path::Path::new(&path)),
                file_path: PathBuf::from(&path),
                metadata: TrackMetadata {
                    title: row.title.map(str::to_string),
                    artist: row.artist.map(str::to_string),
                    album: row.album.map(str::to_string),
                    album_artist: row.album_artist.map(str::to_string),
                    composer: row.composer.map(str::to_string),
                    comment: row.comment.map(str::to_string),
                    genre: row.genre.map(str::to_string),
                    year: row.year,
                    ..TrackMetadata::default()
                },
                duration: row.duration_millis.map(Duration::from_millis),
                sample_rate: row.sample_rate,
                channels: row.channels,
                play_count: row.play_count,
                last_played,
                date_added,
                favorite: row.favorite,
                search_text: String::new(),
            }
        })
        .collect()
}

/// Seed the Rule corpus. The scan batch persists tags, format facts and
/// `date_added`; play history is written by a raw `UPDATE` (the scan commit
/// deliberately never touches it) and the favorite flag through the mutation
/// port, because that is what each fact actually is. `rules_tracks` preserves
/// the corpus order, so a row and its `Track` can be walked together.
pub(crate) fn seeded_rules_store(db_path: &std::path::Path) -> (SqliteStore, Vec<Track>) {
    let now = SystemTime::now();
    let (changes_tx, _changes_rx) =
        crossbeam_channel::unbounded::<riff_persistence::store::StoreChanged>();
    let mut store =
        riff_infra::store::SqliteStore::open_and_migrate(db_path, changes_tx).expect("store opens");
    let offset = local_offset_seconds(&store);
    let rows = rules_fixtures();
    let tracks = rules_tracks(&rows, offset, now);

    store.apply_scan_batch(&tracks).expect("fixtures apply");

    for (row, track) in rows.iter().zip(&tracks) {
        let last_nanos = track.last_played.map(signed_nanos);
        store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE tracks SET play_count = ?1, last_played_nanos = ?2 WHERE path = ?3",
                    rusqlite::params![i64::from(row.play_count), last_nanos, track.id.0],
                )
            })
            .expect("seeding history must work");
    }
    for (row, track) in rows.iter().zip(&tracks) {
        if row.favorite {
            store
                .set_track_favorite(&track.id, true)
                .expect("seeding a favorite must work");
        }
    }
    (store, tracks)
}

/// An independent Rust-only restatement of what a Rule tree *means*, walked
/// over the fixture list with no help from the query compiler: no shared
/// helper, no shared field-id → column table, and no `riff_infra` import at
/// all. The store's compiled predicate must agree with this, Track for Track
/// and in order.
///
/// **THE TRAP, stated because a future refactor will try to unify the two
/// sides.** SQL's `NOT` on a missing value answers *missing*, and a `WHERE`
/// reads missing as false, so an untagged Track silently disappears from a
/// negated Rule — the opposite of the promise. If this reference answered a
/// missing value with `false` (the obvious Rust spelling, and the one that
/// reads as "coalescing is a detail"), it would agree with exactly the broken
/// SQL the compiler must not produce, and the whole arrangement would prove
/// nothing. So the fold is spelled here by hand and by type, and it is
/// *deliberately* the same decision the compiler makes for a different reason:
///
/// * absent **text** is the empty string, at the value — so `is` and `contains`
///   fail it and `is not` takes it in;
/// * absent **number, duration or date** makes the *test* false, and a
///   `column IS NULL` answers `is missing`;
/// * a **Box** of two-valued arms is two-valued, because `and` / `or` here are
///   Rust's, over `bool` — there is no third answer to propagate.
///
/// The same discipline is why the case fold is spelled twice rather than
/// shared: the column side folds ASCII only (what SQLite's `lower()` does,
/// ADR 0020) and the needle side folds fully (what Rust's `to_lowercase()`
/// does). One helper doing both would hide the asymmetry the non-Latin case
/// pins.
/// A property test that cannot turn its counterexample into a fixed case is a
/// liability, so this module is `pub(crate)`: the generated-tree suite
/// (`rule_tree_generator_tests`) walks this same reference and nothing else.
pub(crate) mod rules_reference {
    use super::*;
    use riff_persistence::field_catalog::{self, LOSSLESS_EXTENSIONS};
    use riff_persistence::rule::{Combine, MatchableField, RuleValue, Semantic};

    /// The clock the read is answered at: the instant a `in_last` window counts
    /// back from, and the machine's zone offset a zone-less boundary is read in.
    pub struct Clock {
        pub now: SystemTime,
        pub offset_seconds: i64,
    }

    /// What one field holds for one Track, in the unit the Rule speaks. The
    /// mapping from a stored id to a Track fact is spelled out here by hand,
    /// in this module's own order, because sharing the compiler's column table
    /// would make the comparison a tautology.
    enum Held<'a> {
        /// A tag. `None` is absent and folds to the empty string.
        Tag(Option<&'a str>),
        /// The `;`-separated genre set.
        Set(Option<&'a str>),
        /// A whole number in its own unit.
        Whole(Option<i64>),
        /// A duration, in whole milliseconds.
        Millis(Option<u64>),
        /// A history instant.
        Moment(Option<SystemTime>),
        /// A fact with no absent state.
        Flag(bool),
        /// An id, or an id-and-test pair, this walk cannot read.
        Unreadable,
    }

    /// ASCII-only case fold — SQLite's `lower()`, not Rust's.
    fn sql_lower(text: &str) -> String {
        text.to_ascii_lowercase()
    }

    /// The fold every text test runs on: absent becomes the empty string, and
    /// what is there is folded the way the column is.
    fn folded_text(held: &Held<'_>) -> Option<String> {
        match held {
            Held::Tag(value) | Held::Set(value) => Some(sql_lower(value.unwrap_or(""))),
            _ => None,
        }
    }

    /// What one field id holds for one Track.
    fn held<'a>(field_id: &str, track: &'a Track) -> Held<'a> {
        let metadata = &track.metadata;
        match field_id {
            "genre" => Held::Set(metadata.genre.as_deref()),
            "artist" => Held::Tag(metadata.artist.as_deref()),
            "album_artist" => Held::Tag(metadata.album_artist.as_deref()),
            "album" => Held::Tag(metadata.album.as_deref()),
            "title" => Held::Tag(metadata.title.as_deref()),
            "composer" => Held::Tag(metadata.composer.as_deref()),
            "comment" => Held::Tag(metadata.comment.as_deref()),
            "year" => Held::Whole(metadata.year.map(i64::from)),
            // This side compares whole milliseconds against the Track's own
            // `Duration`; the store compares nanoseconds against a rule value
            // scaled into nanoseconds. The corpus holds whole milliseconds,
            // which is what makes 1 ms = 1e6 ns exact on both sides.
            "duration" => Held::Millis(
                track
                    .duration
                    .map(|length| u64::try_from(length.as_millis()).unwrap_or(u64::MAX)),
            ),
            "play_count" => Held::Whole(Some(i64::from(track.play_count))),
            "last_played" => Held::Moment(track.last_played),
            "date_added" => Held::Moment(track.date_added),
            "favorite" => Held::Flag(track.favorite),
            "sample_rate" => Held::Whole(track.sample_rate.map(i64::from)),
            "channels" => Held::Whole(track.channels.map(i64::from)),
            // A case-insensitive suffix match on the *stored path*, which is
            // what the predicate tests — not the file's extension as `Path`
            // parses it, and no codec is ever read.
            "lossless" => Held::Flag(
                LOSSLESS_EXTENSIONS
                    .iter()
                    .any(|format| track.id.0.to_lowercase().ends_with(&format!(".{format}"))),
            ),
            _ => Held::Unreadable,
        }
    }

    /// One leaf's answer, or `None` when this build cannot read the leaf at all
    /// (an unknown id, a test its field does not permit, a missing value, or a
    /// value of the wrong kind). `None` is a refusal, never an answer: a tree
    /// that cannot be read must not be walked into "nothing matched".
    fn leaf(
        field_id: &str,
        operator_id: &str,
        value: Option<&RuleValue>,
        track: &Track,
        clock: &Clock,
    ) -> Option<bool> {
        let field = MatchableField::from_stored_id(field_id)?;
        let comparison = riff_persistence::rule::Comparison::from_stored_id(operator_id)?;
        let row = field_catalog::spec(field);
        if !row.permits(comparison) {
            return None;
        }
        let held = held(field_id, track);
        if matches!(held, Held::Unreadable) {
            return None;
        }
        let semantic = comparison.semantic();

        // The one test that takes no value.
        if row.value_widgets(comparison).is_empty() {
            return match &held {
                // On text the empty string *is* the absence — one test serves
                // `is ""` and `is missing`, and that is what lets a negated `is`
                // sweep the untagged up.
                Held::Tag(_) | Held::Set(_) => Some(folded_text(&held)?.is_empty()),
                // A number, a duration and a date have no empty value to fold
                // into, so absence is asked of the column itself.
                Held::Whole(number) => Some(number.is_none()),
                Held::Millis(length) => Some(length.is_none()),
                Held::Moment(moment) => Some(moment.is_none()),
                // A flag has no absent state, and a path is always a path: the
                // catalog never offers this row for either.
                Held::Flag(_) | Held::Unreadable => None,
            };
        }

        let value = value?;
        if value.value_type() != row.value_type() {
            return None;
        }
        match (&held, semantic) {
            // ---- text ------------------------------------------------------
            (Held::Tag(_), Semantic::Equality) => {
                let needle = text(value)?.to_lowercase();
                Some(folded_text(&held)? == needle)
            }
            (Held::Set(_), Semantic::Equality) => {
                // One whole segment: split on `;`, strip spaces from both sides,
                // and compare — which is exactly what the store's
                // delimiter-wrapped `instr` asks for, arrived at differently.
                let target = text(value)?.to_lowercase().replace(' ', "");
                Some(
                    folded_text(&held)?
                        .replace(' ', "")
                        .split(';')
                        .any(|segment| segment == target),
                )
            }
            (Held::Tag(_) | Held::Set(_), Semantic::Containment) => {
                let needle = text(value)?.to_lowercase();
                Some(folded_text(&held)?.contains(&needle))
            }

            // ---- numbers ---------------------------------------------------
            // Both arms fold the same way and are spelled twice on purpose: an
            // absent column makes the *test* false, so a negated test is true
            // and the Track stays in. That sentence is the whole promise, and it
            // is what a `held.unwrap_or_default()` spelling would quietly break
            // (an absent duration would read as zero seconds long).
            (Held::Whole(number), _) => Some(numeric(*number, semantic, number_value(value)?)),
            (Held::Millis(length), _) => Some(numeric(*length, semantic, duration_value(value)?)),

            // ---- history ---------------------------------------------------
            (Held::Moment(moment), _) => match semantic {
                Semantic::WithinDays => {
                    let days = days_value(value)?;
                    let cutoff = clock
                        .now
                        .checked_sub(Duration::from_secs(u64::from(days).saturating_mul(86_400)))
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    Some(moment.is_some_and(|played| played >= cutoff))
                }
                Semantic::EarlierThan | Semantic::LaterThan => {
                    // Second granularity, because that is all a zone-less
                    // boundary carries: a stamp inside the boundary's own
                    // second is neither before nor after it.
                    let instant = date_value(value)?;
                    let boundary = epoch_seconds(&local_instant(instant, clock.offset_seconds));
                    let Some(played) = moment else {
                        return Some(false);
                    };
                    let played = epoch_seconds(played);
                    Some(match semantic {
                        Semantic::EarlierThan => played < boundary,
                        _ => played > boundary,
                    })
                }
                _ => None,
            },

            // ---- yes / no --------------------------------------------------
            (Held::Flag(flag), Semantic::Equality) => Some(*flag == yes_no(value)?),
            _ => None,
        }
    }

    /// One Box, and everything under it, answered for one Track.
    fn answers(tree: &RuleTree, track: &Track, clock: &Clock) -> Option<bool> {
        match tree {
            RuleTree::Rule {
                field_id,
                operator_id,
                negated,
                value,
            } => {
                let answer = leaf(field_id, operator_id, value.as_ref(), track, clock)?;
                Some(if *negated { !answer } else { answer })
            }
            RuleTree::Box { combine, children } => {
                let combine = Combine::from_stored_id(combine)?;
                if children.is_empty() {
                    return None;
                }
                let mut arms = Vec::with_capacity(children.len());
                for child in children {
                    arms.push(answers(child, track, clock)?);
                }
                Some(match combine {
                    Combine::All => arms.iter().all(|arm| *arm),
                    Combine::Any => arms.iter().any(|arm| *arm),
                })
            }
        }
    }

    /// Whether this walk can read the whole tree — the reference's own share of
    /// the refusal the compiler makes before producing SQL.
    pub fn can_read(tree: &RuleTree, clock: &Clock) -> bool {
        // Any Track will do: readability is a question about the tree, not the
        // row, and the leaf walk answers both at once.
        let probe = Track {
            id: TrackId("probe".to_string()),
            file_path: PathBuf::from("probe.mp3"),
            metadata: TrackMetadata::default(),
            duration: None,
            sample_rate: None,
            channels: None,
            play_count: 0,
            last_played: None,
            date_added: None,
            favorite: false,
            search_text: String::new(),
        };
        answers(tree, &probe, clock).is_some()
    }

    /// Every Track the tree matches, in the store's canonical flat order (path
    /// ascending, byte-wise), which is the order the read is asked for.
    pub fn matches(
        tracks: &[Track],
        tree: &RuleTree,
        needle: Option<&str>,
        clock: &Clock,
    ) -> Vec<TrackId> {
        let mut ids: Vec<TrackId> = tracks
            .iter()
            .filter(|track| {
                answers(tree, track, clock)
                    .expect("the tree must be readable before its answer is asked for")
                // The needle is ANDed with the tree, never chosen against it.
                && track
                    .metadata
                    .search_text()
                    .contains(&needle.map_or_else(String::new, str::to_lowercase))
            })
            .map(|track| track.id.clone())
            .collect();
        ids.sort_by(|left, right| left.0.cmp(&right.0));
        ids
    }

    /// One ordering test over a nullable numeric column, folded the way the
    /// compiler folds it: no value, no truth.
    fn numeric<T: Ord + Copy>(held: Option<T>, semantic: Semantic, target: T) -> bool {
        held.is_some_and(|value| ordered_with(value, semantic, target))
    }

    fn ordered_with<T: Ord>(held: T, semantic: Semantic, target: T) -> bool {
        match semantic {
            Semantic::GreaterThan => held > target,
            Semantic::LessThan => held < target,
            _ => held == target,
        }
    }

    /// Whole seconds since the epoch, floored — the same resolution a
    /// `strftime('%Y-%m-%d %H:%M:%S')` rendering carries.
    fn epoch_seconds(time: &SystemTime) -> i64 {
        match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(forward) => i64::try_from(forward.as_secs()).unwrap_or(i64::MAX),
            Err(back) => -(i64::try_from(back.duration().as_secs()).unwrap_or(i64::MAX)),
        }
    }

    fn text(value: &RuleValue) -> Option<&str> {
        match value {
            RuleValue::Text(text) => Some(text),
            _ => None,
        }
    }

    fn number_value(value: &RuleValue) -> Option<i64> {
        match value {
            RuleValue::Number(number) => Some(*number),
            _ => None,
        }
    }

    fn duration_value(value: &RuleValue) -> Option<u64> {
        match value {
            RuleValue::DurationMillis(millis) => Some(*millis),
            _ => None,
        }
    }

    fn days_value(value: &RuleValue) -> Option<u32> {
        match value {
            RuleValue::Days(days) => Some(*days),
            _ => None,
        }
    }

    fn date_value(value: &RuleValue) -> Option<&CivilDateTime> {
        match value {
            RuleValue::DateTime(instant) => Some(instant),
            _ => None,
        }
    }

    fn yes_no(value: &RuleValue) -> Option<bool> {
        match value {
            RuleValue::YesNo(answer) => Some(*answer),
            _ => None,
        }
    }
}

// --- Rule-tree case builders ----------------------------------------------
//
// These build the *input* the two sides share, exactly as `smart_fixtures` is
// the shared input of the built-in lists: data, no logic, and no field id that
// the compiler's own table owns.
//
// `pub(crate)` because ticket 10's generated trees are built with these and
// printed with these: a counterexample the property test reports is pasted into
// a fixed case as `all_of(vec![rule(…)])` text, and that only round-trips if
// the fixed case reaches for the same builders the generator used.

#[allow(
    clippy::unnecessary_wraps,
    reason = "a probe names its value in one call"
)]
pub(crate) fn needle(text: &str) -> Option<RuleValue> {
    Some(RuleValue::Text(text.to_string()))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "a probe names its value in one call"
)]
pub(crate) fn number(value: i64) -> Option<RuleValue> {
    Some(RuleValue::Number(value))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "a probe names its value in one call"
)]
pub(crate) fn millis(value: u64) -> Option<RuleValue> {
    Some(RuleValue::DurationMillis(value))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "a probe names its value in one call"
)]
pub(crate) fn days(value: u32) -> Option<RuleValue> {
    Some(RuleValue::Days(value))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "a probe names its value in one call"
)]
pub(crate) fn boundary(instant: CivilDateTime) -> Option<RuleValue> {
    Some(RuleValue::DateTime(instant))
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "a probe names its value in one call"
)]
pub(crate) fn yes(value: bool) -> Option<RuleValue> {
    Some(RuleValue::YesNo(value))
}

/// One Rule, in the store's own spelling, at the given polarity.
pub(crate) fn rule(
    field: MatchableField,
    comparison: Comparison,
    value: Option<RuleValue>,
) -> RuleTree {
    RuleTree::Rule {
        field_id: field.stored_id().to_string(),
        operator_id: comparison.stored_id().to_string(),
        negated: false,
        value,
    }
}

/// The same test inverted — the one spelling of "is not" there is.
pub(crate) fn rule_not(
    field: MatchableField,
    comparison: Comparison,
    value: Option<RuleValue>,
) -> RuleTree {
    RuleTree::Rule {
        field_id: field.stored_id().to_string(),
        operator_id: comparison.stored_id().to_string(),
        negated: true,
        value,
    }
}

pub(crate) fn all_of(children: Vec<RuleTree>) -> RuleTree {
    RuleTree::Box {
        combine: "all".to_string(),
        children,
    }
}

pub(crate) fn any_of(children: Vec<RuleTree>) -> RuleTree {
    RuleTree::Box {
        combine: "any".to_string(),
        children,
    }
}

/// A Rule tree in the store's spelling, built from a Root Box the frontend
/// would hold. Used by the cases that must prove the two spellings agree.
fn from_root(boxed: riff_persistence::rule::RootBox) -> RuleTree {
    RuleTree::from_root_box(&boxed)
}

/// One probe value per case the matrix asks: a needle, a fragment, a threshold,
/// a window or a boundary that this corpus *splits*, so a case that silently
/// answered "everything" or "nothing" would be visible in the fixture rather
/// than hidden in the assertion. The catalog still decides which comparisons are
/// asked at all — this table only answers what value to write, and a shared
/// threshold under two comparisons is deliberate: it proves `>` and `<` are two
/// tests of one number rather than one test written twice.
fn probe(field: MatchableField, comparison: Comparison) -> Option<RuleValue> {
    use Comparison as C;
    use MatchableField as F;
    match field {
        F::Genre => match comparison {
            C::Is => needle("Progressive Rock"),
            C::Contains => needle("rock;progressive"),
            _ => None,
        },
        F::Artist => match comparison {
            C::Is => needle("Unknown Artist"),
            C::Contains => needle("known"),
            _ => None,
        },
        F::AlbumArtist => match comparison {
            C::Is => needle("AA"),
            C::Contains => needle("e"),
            _ => None,
        },
        F::Album => match comparison {
            C::Is => needle("First"),
            C::Contains => needle("eo"),
            _ => None,
        },
        F::Title => match comparison {
            C::Is => needle("Alpha"),
            C::Contains => needle("lph"),
            _ => None,
        },
        F::Composer => match comparison {
            C::Is => needle("Bach"),
            C::Contains => needle("iva"),
            _ => None,
        },
        F::Comment => match comparison {
            C::Is => needle("notes"),
            C::Contains => needle("liner"),
            _ => None,
        },
        F::Year => match comparison {
            C::EqualTo => number(1975),
            C::GreaterThan | C::LessThan => number(1990),
            _ => None,
        },
        // The round-millisecond pair: exactly one hour, and one millisecond
        // over it. Equality and the ordering land on opposite rows.
        F::Duration => match comparison {
            C::EqualTo | C::GreaterThan => millis(3_600_000),
            C::LessThan => millis(300_000),
            _ => None,
        },
        F::PlayCount => match comparison {
            C::EqualTo | C::GreaterThan | C::LessThan => number(3),
            _ => None,
        },
        // The local-midnight boundary: one row's stamp is 23:30 the evening
        // before it and one is 00:30 the morning after.
        F::LastPlayed => match comparison {
            C::InLast => days(7),
            C::Before | C::After => boundary(civil(2024, 3, 2, 0, 0, 0)),
            _ => None,
        },
        F::DateAdded => match comparison {
            C::InLast => days(30),
            C::Before | C::After => boundary(civil(2024, 6, 2, 0, 0, 0)),
            _ => None,
        },
        // Both flag fields are probed with "yes", and the matrix asks each one
        // negated as well, which is the other side.
        F::Favorite | F::Lossless => match comparison {
            C::Is => yes(true),
            _ => None,
        },
        F::SampleRate => match comparison {
            C::EqualTo | C::LessThan => number(44_100),
            C::GreaterThan => number(48_000),
            _ => None,
        },
        F::Channels => match comparison {
            C::EqualTo | C::GreaterThan | C::LessThan => number(2),
            _ => None,
        },
    }
}

// --- Rule-tree reads -------------------------------------------------------

/// The Rule listing's window, unwrapped, in the canonical flat order.
pub(crate) fn rule_window(
    store: &SqliteStore,
    tree: &RuleTree,
    needle_text: Option<&str>,
) -> Vec<TrackId> {
    let page =
        store.custom_smart_playlist_page(tree, TrackListOrder::PathAsc, needle_text, 0, usize::MAX);
    match page {
        Ok(page) => page.rows().iter().map(|track| track.id.clone()).collect(),
        Err(error) => panic!("the Rule tree must be answerable: {error}"),
    }
}

/// The Rule listing's total, unwrapped — the same Listing Page's other half.
pub(crate) fn rule_total(store: &SqliteStore, tree: &RuleTree, needle_text: Option<&str>) -> usize {
    store
        .custom_smart_playlist_page(tree, TrackListOrder::PathAsc, needle_text, 0, usize::MAX)
        .expect("the Rule tree must be answerable")
        .total()
}

/// The clock a reference walk answers at: the instant `in_last` counts back
/// from, and the machine's own zone, which is what a zone-less boundary means.
pub(crate) fn rule_clock(store: &SqliteStore) -> rules_reference::Clock {
    rules_reference::Clock {
        now: SystemTime::now(),
        offset_seconds: local_offset_seconds(store),
    }
}

/// The one assertion every Rule case makes: the store's compiled predicate
/// answers exactly what the independent walk answers — Track identity in the
/// canonical order, and a total that counts the rows the window came from.
/// Never SQL text, never table internals.
pub(crate) fn assert_rule_membership(
    store: &SqliteStore,
    tracks: &[Track],
    tree: &RuleTree,
    needle_text: Option<&str>,
    clock: &rules_reference::Clock,
    why: &str,
) {
    let expected = rules_reference::matches(tracks, tree, needle_text, clock);
    assert_eq!(
        rule_window(store, tree, needle_text),
        expected,
        "{why}: the store's answer must be the reference's"
    );
    assert_eq!(
        rule_total(store, tree, needle_text),
        expected.len(),
        "{why}: the total must count the same rows the window came from"
    );
}

#[test]
fn test_every_field_and_comparison_the_catalog_permits_matches_the_reference() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    let mut cases = 0;
    for field in MatchableField::ALL {
        let row = riff_persistence::field_catalog::spec(field);
        for comparison in row.comparisons() {
            // The catalog's own widget answer decides whether a value is
            // written at all — the matrix never restates it.
            let draws_a_value = !row.value_widgets(*comparison).is_empty();
            let value = probe(field, *comparison);
            assert_eq!(
                value.is_some(),
                draws_a_value,
                "the probe table must write a value for exactly the cases \
                 the catalog draws one for: {field:?} / {comparison:?}"
            );
            for negated in [false, true] {
                let tree = if negated {
                    rule_not(field, *comparison, value.clone())
                } else {
                    rule(field, *comparison, value.clone())
                };
                assert!(
                    rules_reference::can_read(&tree, &clock),
                    "a case the catalog permits must be readable by both sides: {tree:?}"
                );
                let polarity = if negated { "is not" } else { "is" };
                assert_rule_membership(
                    &store,
                    &tracks,
                    &tree,
                    None,
                    &clock,
                    &format!("{} {polarity} {:?}", row.display_name(), *comparison),
                );
                cases += 1;
            }
        }
    }
    assert!(
        cases >= 70,
        "sixteen fields across every comparison they permit, both polarities, \
         is at least seventy cases — the matrix silently skipped rows: {cases}"
    );
}

/// The file name a `TrackId` names, for a failure message a person can read.
pub(crate) fn rule_name(id: &TrackId) -> &str {
    id.0.rsplit('\\').next().unwrap_or(&id.0)
}

/// The negation pins: each entry is a tree, the corpus names it must answer,
/// and why the shape is on the list. Pulled out of the test because the table
/// is the point and a wall of fifteen-row vectors is not a readable assertion.
#[allow(
    clippy::too_many_lines,
    reason = "one pinned table: a tree per absent shape"
)]
fn negation_pins() -> Vec<(&'static str, RuleTree, Vec<&'static str>)> {
    vec![
        (
            "duration is not greater than one hour",
            rule_not(
                MatchableField::Duration,
                Comparison::GreaterThan,
                millis(3_600_000),
            ),
            vec![
                "aged.mp3",
                "eight.mp3",
                "eleven.mp3",
                "five.FLAC",
                "four.m4a",
                "nine.mp3",
                "oddball.mp3",
                "one.flac",
                "quote.mp3",
                "recent.mp3",
                "seven.mp3",
                "six",
                "ten.mp3",
                "two.wav",
            ],
        ),
        (
            "year is missing",
            rule(MatchableField::Year, Comparison::IsMissing, None),
            vec!["eleven.mp3", "six"],
        ),
        (
            "year is not 1975",
            rule_not(MatchableField::Year, Comparison::EqualTo, number(1975)),
            vec![
                "aged.mp3",
                "eight.mp3",
                "eleven.mp3",
                "five.FLAC",
                "nine.mp3",
                "oddball.mp3",
                "quote.mp3",
                "recent.mp3",
                "six",
                "ten.mp3",
                "three.mp3",
                "two.wav",
            ],
        ),
        (
            "sample rate is not 44100",
            rule_not(
                MatchableField::SampleRate,
                Comparison::EqualTo,
                number(44_100),
            ),
            vec![
                "eleven.mp3",
                "four.m4a",
                "nine.mp3",
                "six",
                "ten.mp3",
                "two.wav",
            ],
        ),
        (
            "channels is not 2",
            rule_not(MatchableField::Channels, Comparison::EqualTo, number(2)),
            vec![
                "aged.mp3",
                "eleven.mp3",
                "four.m4a",
                "nine.mp3",
                "six",
                "ten.mp3",
                "two.wav",
            ],
        ),
        (
            "last played is not in the last 7 days",
            rule_not(MatchableField::LastPlayed, Comparison::InLast, days(7)),
            vec![
                "aged.mp3",
                "eight.mp3",
                "eleven.mp3",
                "five.FLAC",
                "four.m4a",
                "nine.mp3",
                "oddball.mp3",
                "one.flac",
                "quote.mp3",
                "seven.mp3",
                "six",
                "ten.mp3",
                "three.mp3",
                "two.wav",
            ],
        ),
        (
            "date added is missing",
            rule(MatchableField::DateAdded, Comparison::IsMissing, None),
            vec!["eleven.mp3", "six"],
        ),
    ]
}

#[test]
fn test_negated_rules_include_tracks_whose_value_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    // Text: absent is the empty string, so `is not` sweeps the untagged up.
    let not_alpha = rule_not(MatchableField::Title, Comparison::Is, needle("Alpha"));
    assert_rule_membership(
        &store,
        &tracks,
        &not_alpha,
        None,
        &clock,
        "title is not Alpha",
    );
    let mut expect: Vec<&str> = rules_fixtures()
        .iter()
        .map(|row| row.name)
        .filter(|name| *name != "one.flac")
        .collect();
    expect.sort_unstable();
    assert!(
        expect.contains(&"eleven.mp3") && expect.contains(&"six"),
        "the corpus must hold absent-title rows for this to mean anything"
    );
    let names: Vec<String> = rule_window(&store, &not_alpha, None)
        .iter()
        .map(rule_name)
        .map(str::to_string)
        .collect();
    assert_eq!(
        names,
        expect.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "`title is not \"Alpha\"` is every other Track, absent title included — \
         `eleven.mp3` carries no title at all, which is exactly what a bare \
         `NOT (lower(title) = 'alpha')` in SQL would have dropped"
    );

    // Numbers and dates: no empty value exists, so the TEST is folded, and a
    // negated test on an absent column is true.
    for (why, tree, mut names) in negation_pins() {
        names.sort_unstable();
        assert_rule_membership(&store, &tracks, &tree, None, &clock, why);
        let actual: Vec<String> = rule_window(&store, &tree, None)
            .iter()
            .map(rule_name)
            .map(str::to_string)
            .collect();
        assert_eq!(
            actual,
            names.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "{why}: pinned as membership, and every absent column must be inside \
             a negated numeric or date test"
        );
    }
}

#[test]
fn test_box_mixing_an_unknown_and_a_false_arm_still_includes_the_track() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    // The arm that would answer *unknown* without the fold: a duration test on
    // a Track with no duration. Beside it, an arm that answers false. Unfolded,
    // `false OR unknown` is unknown, and `WHERE unknown` is false, so
    // `eleven.mp3` (no title, no duration) would vanish from a Box whose
    // negation says it belongs.
    let tree = all_of(vec![any_of(vec![
        rule_not(MatchableField::Title, Comparison::Is, needle("Alpha")),
        rule_not(
            MatchableField::Duration,
            Comparison::GreaterThan,
            millis(3_600_000),
        ),
    ])]);
    assert_rule_membership(&store, &tracks, &tree, None, &clock, "negated Box arms");
    let ids = rule_window(&store, &tree, None);
    assert!(
        ids.contains(&TrackId::from_path(std::path::Path::new(
            "r:\\rules\\eleven.mp3"
        ))),
        "a Track with neither a title nor a duration is inside a Box whose arms \
         are one false and one would-be-unknown answer: {:?}",
        ids.iter().map(rule_name).collect::<Vec<_>>()
    );

    // The same arms without the negation: both fold to false, so the Box is
    // false and the Track stays out — the fold is two-valued, not permissive.
    let positive = any_of(vec![
        rule(MatchableField::Title, Comparison::Is, needle("Alpha")),
        rule(
            MatchableField::Duration,
            Comparison::GreaterThan,
            millis(3_600_000),
        ),
    ]);
    assert_rule_membership(
        &store,
        &tracks,
        &positive,
        None,
        &clock,
        "positive Box arms",
    );
    assert_eq!(
        rule_window(&store, &positive, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["one.flac", "three.mp3"],
        "`Alpha` or longer than an hour: the tagged long Track and the \
         one-millisecond-over one, and no absent-duration row"
    );
}

#[test]
fn test_genre_is_one_whole_segment_and_a_tab_padded_segment_is_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    // A `;`-separated tag: `is` picks one whole segment, so "Rock" alone is not
    // a segment of "Art Rock;Progressive Rock".
    let is_rock = rule(MatchableField::Genre, Comparison::Is, needle("Rock"));
    assert_rule_membership(&store, &tracks, &is_rock, None, &clock, "genre is Rock");
    assert!(
        rule_window(&store, &is_rock, None).is_empty(),
        "no Track carries `Rock` as a whole segment"
    );

    let is_progressive = rule(
        MatchableField::Genre,
        Comparison::Is,
        needle("Progressive Rock"),
    );
    assert_rule_membership(
        &store,
        &tracks,
        &is_progressive,
        None,
        &clock,
        "genre is Progressive Rock",
    );
    assert_eq!(
        rule_window(&store, &is_progressive, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["one.flac"],
        "THE DOCUMENTED DISAGREEMENT, pinned rather than hidden: `two.wav` tags \
         itself \"Art Rock;<TAB>Progressive Rock\", and a Rule strips only \
         SPACES, so the tab-padded segment is not `progressiverock` and does not \
         match — while the genres view trims each segment and shows it. \
         A Rule and a view disagree on purpose."
    );
    let genres = store.genre_counts().expect("genre counts");
    assert!(
        genres
            .iter()
            .any(|count| count.genre == "Progressive Rock" && count.tracks == 2),
        "the genres view must show that tag twice, which is the disagreement: {genres:?}"
    );

    // Space-padded segments match, because spaces are stripped on both sides.
    let padded = rule(
        MatchableField::Genre,
        Comparison::Is,
        needle("  Art  Rock "),
    );
    assert_rule_membership(&store, &tracks, &padded, None, &clock, "padded needle");
    assert_eq!(
        rule_window(&store, &padded, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["one.flac", "two.wav"],
        "spaces are stripped from the tag and from the needle, so both rows' \
         first segment answers"
    );

    // An empty-string genre and a NULL genre both answer "missing".
    let missing = rule(MatchableField::Genre, Comparison::IsMissing, needle(""));
    assert_rule_membership(&store, &tracks, &missing, None, &clock, "genre missing");
    assert_eq!(
        rule_window(&store, &missing, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["eleven.mp3", "four.m4a", "three.mp3"],
        "an empty genre and an absent genre are the same absence"
    );
    let empty_is = rule(MatchableField::Genre, Comparison::Is, needle(""));
    assert_eq!(
        rule_window(&store, &empty_is, None),
        rule_window(&store, &missing, None),
        "`genre is \"\"` and `genre is missing` are one test on text"
    );
}

#[test]
fn test_lossless_is_a_case_insensitive_suffix_match_on_the_stored_path() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    let lossless = rule(MatchableField::Lossless, Comparison::Is, yes(true));
    assert_rule_membership(&store, &tracks, &lossless, None, &clock, "lossless is yes");
    assert_eq!(
        rule_window(&store, &lossless, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["five.FLAC", "one.flac", "two.wav"],
        "an UPPERCASE `.FLAC` is lossless (a suffix match on the lowered path), \
         and a file with NO extension is not"
    );

    let lossy = rule(MatchableField::Lossless, Comparison::Is, yes(false));
    assert_rule_membership(&store, &tracks, &lossy, None, &clock, "lossless is no");
    let lossy_names: Vec<String> = rule_window(&store, &lossy, None)
        .iter()
        .map(rule_name)
        .map(str::to_string)
        .collect();
    assert!(
        lossy_names.contains(&"four.m4a".to_string()) && lossy_names.contains(&"six".to_string()),
        "`.m4a` is PINED as not-lossless (the real codec is never read, so an \
         ALAC in an m4a container reports as lossy) and `six` has no extension: {lossy_names:?}"
    );
    assert_eq!(
        lossy_names.len() + 3,
        15,
        "every Track lands in exactly one of the two answers"
    );

    // The two literal lists classify every extension the Library can index,
    // and never both. Restated here at the store seam because this is where a
    // mis-scaled list would show up as a wrong membership answer.
    let mut both = LOSSLESS_EXTENSIONS
        .iter()
        .chain(LOSSY_EXTENSIONS)
        .collect::<Vec<_>>();
    both.sort();
    let distinct: Vec<&&str> = both
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        distinct.len(),
        both.len(),
        "a format may not be in both lists at once"
    );
}

#[test]
fn test_non_latin_tags_do_not_fold_case_and_that_is_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    // Two rows, the same tag in two cases, one of them non-Latin. SQLite's
    // `lower()` folds ASCII only (ADR 0020) while the needle is folded by Rust,
    // so the accented UPPERCASE row is unreachable — the accepted limitation,
    // pinned as membership rather than hidden behind a helper.
    let is_electronic = rule(
        MatchableField::Genre,
        Comparison::Is,
        needle("Électronique"),
    );
    assert_rule_membership(
        &store,
        &tracks,
        &is_electronic,
        None,
        &clock,
        "genre is Électronique",
    );
    assert_eq!(
        rule_window(&store, &is_electronic, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["ten.mp3"],
        "only the lowercase row answers: `nine.mp3` tags itself ÉLECTRONIQUE and \
         no fold touches the accent, so a listener cannot reach it by that name"
    );

    // The ASCII part of the same name still reaches the ASCII part of the tag.
    let contains_tronic = rule(
        MatchableField::Genre,
        Comparison::Contains,
        needle("tronique"),
    );
    assert_rule_membership(
        &store,
        &tracks,
        &contains_tronic,
        None,
        &clock,
        "genre contains tronique",
    );
    assert_eq!(
        rule_window(&store, &contains_tronic, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["nine.mp3", "ten.mp3"],
        "both cases answer an ASCII needle, which is exactly how far the fold goes"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one narrative: ANDing, literal needles, then paging"
)]
fn test_the_needle_is_anded_inside_the_read_and_paging_never_repeats_or_skips() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    let ambient = rule(MatchableField::Genre, Comparison::Is, needle("Ambient"));
    assert_rule_membership(&store, &tracks, &ambient, None, &clock, "genre is Ambient");
    assert_eq!(
        rule_window(&store, &ambient, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["aged.mp3", "five.FLAC", "recent.mp3", "six"],
        "the tree alone answers four Tracks"
    );

    // ANDed, never chosen between: the same tree with a needle the tree also
    // permits narrows the answer instead of replacing it.
    assert_eq!(
        rule_window(&store, &ambient, Some("oscar"))
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["aged.mp3"],
        "the needle lands inside the tree's answer"
    );
    assert_rule_membership(
        &store,
        &tracks,
        &ambient,
        Some("oscar"),
        &clock,
        "ANDed needle",
    );
    assert_eq!(
        (
            rule_total(&store, &ambient, Some("oscar")),
            rule_total(&store, &ambient, None)
        ),
        (1, 4),
        "the total counts the filtered rows, not the tree's rows"
    );
    assert_eq!(
        rule_window(&store, &ambient, Some("alpha")),
        Vec::<TrackId>::new(),
        "a needle that matches a Track the tree excludes answers nothing: the \
         two conditions are ANDed, never ORed and never applied after paging"
    );

    // A needle carrying a quote, a `%` and a `_` matches literally.
    let percent = rule(MatchableField::Title, Comparison::Contains, needle("%"));
    assert_rule_membership(&store, &tracks, &percent, None, &clock, "title contains %");
    assert_eq!(
        rule_window(&store, &percent, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["oddball.mp3", "quote.mp3"],
        "a `%` is a character, not a wildcard"
    );
    let underscore = rule(
        MatchableField::Title,
        Comparison::Contains,
        needle("%_done"),
    );
    assert_rule_membership(
        &store,
        &tracks,
        &underscore,
        None,
        &clock,
        "title contains %_done",
    );
    assert_eq!(
        rule_window(&store, &underscore, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["oddball.mp3"],
        "`%_` inside a needle is two literal characters — the shape a `LIKE` \
         would read as \"anything then any one character\""
    );
    let quoted = rule(
        MatchableField::Title,
        Comparison::Is,
        needle("He said \"yes\" 100%"),
    );
    assert_rule_membership(&store, &tracks, &quoted, None, &clock, "a quoted needle");
    assert_eq!(
        rule_window(&store, &quoted, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["quote.mp3"],
        "a quote in the value is data, and it reaches the row that holds it"
    );
    assert_eq!(
        rule_window(&store, &quoted, Some("said")),
        rule_window(&store, &quoted, None),
        "a needle the matched row already satisfies leaves the answer alone"
    );

    // The paged contract: a disjoint, correctly ordered slice and one total.
    let lossy = rule(MatchableField::Lossless, Comparison::Is, yes(false));
    let full = rules_reference::matches(&tracks, &lossy, None, &clock);
    assert_eq!(full.len(), 12, "twelve of the fifteen are not lossless");
    for order in [TrackListOrder::PathAsc, TrackListOrder::TitleAsc] {
        let mut seen = Vec::new();
        for offset in [0usize, 5, 10] {
            let page = store
                .custom_smart_playlist_page(&lossy, order, None, offset, 5)
                .expect("a legal tree is answerable");
            assert_eq!(
                page.total(),
                full.len(),
                "the total does not move with the offset"
            );
            let ids: Vec<TrackId> = page.rows().iter().map(|track| track.id.clone()).collect();
            assert!(
                !ids.is_empty(),
                "offset {offset} of a twelve-row answer has rows"
            );
            seen.extend(ids);
        }
        assert_eq!(seen.len(), 12, "three windows of five over twelve rows");
        assert_eq!(
            seen.iter().collect::<std::collections::HashSet<_>>().len(),
            12,
            "no row appears twice across the windows"
        );
    }

    // The canonical orders end on `path`, so the flat order is the whole
    // listing's total order and the reversal is exact.
    let ascending = rule_window(&store, &lossy, None);
    let descending: Vec<TrackId> = store
        .custom_smart_playlist_page(&lossy, TrackListOrder::PathDesc, None, 0, usize::MAX)
        .expect("a legal tree is answerable")
        .rows()
        .iter()
        .map(|track| track.id.clone())
        .collect();
    assert_eq!(
        descending,
        ascending.iter().rev().cloned().collect::<Vec<_>>(),
        "path-descending is the exact reversal, because path alone is unique"
    );

    // A count read is the same Listing Page asked for zero rows (ADR 0003's
    // 2026-09-29 amendment): total only, empty window.
    let counted = store
        .custom_smart_playlist_page(&lossy, TrackListOrder::PathAsc, None, 0, 0)
        .expect("a count read is a page with no rows");
    assert_eq!((counted.total(), counted.rows().len()), (12, 0));
    let counted_with_needle = store
        .custom_smart_playlist_page(&lossy, TrackListOrder::PathAsc, Some("oscar"), 0, 0)
        .expect("a count read carries the needle too");
    assert_eq!(counted_with_needle.total(), 1);
}

#[test]
fn test_a_tree_whose_rules_match_nothing_answers_empty_and_not_everything() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    let nothing = rule(
        MatchableField::Title,
        Comparison::Is,
        needle("no such title"),
    );
    assert_rule_membership(&store, &tracks, &nothing, None, &clock, "no such title");
    assert_eq!(
        (
            rule_window(&store, &nothing, None),
            rule_total(&store, &nothing, None)
        ),
        (Vec::<TrackId>::new(), 0),
        "a listing that matched nothing is empty, with a total of zero"
    );

    // Two Box levels whose arms cannot both hold: an `all` over an `any` that
    // only ever answers false. The fold must not turn unsatisfiable into
    // everything.
    let unsatisfiable = all_of(vec![
        any_of(vec![
            rule(MatchableField::Title, Comparison::Is, needle("nope")),
            rule(MatchableField::Title, Comparison::Is, needle("also nope")),
        ]),
        rule(MatchableField::Year, Comparison::GreaterThan, number(3000)),
    ]);
    assert_rule_membership(
        &store,
        &tracks,
        &unsatisfiable,
        None,
        &clock,
        "unsatisfiable tree",
    );
    assert_eq!(
        rule_total(&store, &unsatisfiable, None),
        0,
        "a tree no Track can satisfy answers nothing, at every level of it"
    );
}

#[test]
fn test_seven_levels_of_boxes_answer_the_reference_and_the_store_never_counts_them() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    // The editor's ceiling is seven levels; the store counts none of them, so
    // a tree that deep is answered here rather than refused, and a deeper one
    // from a newer riff is answered too (only the engine's own limits bite).
    let mut tree = any_of(vec![
        rule(MatchableField::Composer, Comparison::Is, needle("Bach")),
        rule(
            MatchableField::Duration,
            Comparison::LessThan,
            millis(200_000),
        ),
    ]);
    for level in 0..6 {
        tree = if level % 2 == 0 {
            all_of(vec![tree])
        } else {
            any_of(vec![tree])
        };
    }
    assert_rule_membership(&store, &tracks, &tree, None, &clock, "seven levels");
    assert_eq!(
        rule_window(&store, &tree, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec![
            "aged.mp3",
            "five.FLAC",
            "four.m4a",
            "oddball.mp3",
            "one.flac",
            "quote.mp3",
            "recent.mp3",
        ],
        "Bach or shorter than two minutes, wrapped in six alternating Box levels"
    );

    // A Box beside a Rule under one parent, and a Box inside a Box: the two
    // shapes the editor draws as one interleaved list.
    let mixed = all_of(vec![
        rule(MatchableField::Year, Comparison::EqualTo, number(1975)),
        any_of(vec![
            rule(MatchableField::Favorite, Comparison::Is, yes(true)),
            all_of(vec![
                rule(MatchableField::Channels, Comparison::EqualTo, number(1)),
                rule_not(
                    MatchableField::Genre,
                    Comparison::Is,
                    needle("électronique"),
                ),
            ]),
        ]),
    ]);
    assert_rule_membership(&store, &tracks, &mixed, None, &clock, "mixed siblings");
    assert_eq!(
        rule_window(&store, &mixed, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["four.m4a", "one.flac", "seven.mp3"],
        "1975, and either a favorite or a mono Track that is not the lowercase \
         electronic tag"
    );

    // The store's spelling and the domain's Root Box are one tree: the read is
    // driven from a frontend-shaped value in exactly the case above.
    let boxed = riff_persistence::rule::RootBox::new(
        riff_persistence::rule::RuleBox::new(riff_persistence::rule::Combine::All).with_children(
            vec![riff_persistence::rule::RuleChild::Rule(
                riff_persistence::rule::Rule::new(
                    MatchableField::Year,
                    Comparison::EqualTo,
                    number(1975),
                ),
            )],
        ),
    );
    let spelled = from_root(boxed.clone());
    assert_eq!(
        rule_window(&store, &spelled, None),
        rule_window(
            &store,
            &rule(MatchableField::Year, Comparison::EqualTo, number(1975)),
            None
        ),
        "`RuleTree::from_root_box` writes the ids the compiler reads"
    );
    assert_eq!(
        RuleTree::from_rule_box(boxed.inner()),
        spelled,
        "the Root Box and its inner Box spell the same tree"
    );
    assert_rule_membership(&store, &tracks, &spelled, None, &clock, "from a Root Box");
}

#[test]
fn test_a_zone_less_boundary_is_read_against_the_machines_own_local_midnight() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    // The corpus places `one.flac` at 23:30 on the evening of 1 March and
    // `two.wav` at 00:30 the next morning, *local*, and `four.m4a` weeks
    // earlier. The boundary is the zone-less string a listener would have
    // typed. On a machine set to UTC the two halves are 23:30 and 00:30 UTC and
    // the answer is the same; on any machine with an offset the pair brackets
    // that machine's own midnight rather than the epoch boundary, which is what
    // "read against the listener's own clock on that machine" promises.
    let midnight = civil(2024, 3, 2, 0, 0, 0);
    let before = rule(
        MatchableField::LastPlayed,
        Comparison::Before,
        boundary(midnight),
    );
    assert_rule_membership(
        &store,
        &tracks,
        &before,
        None,
        &clock,
        "before local midnight",
    );
    assert_eq!(
        rule_window(&store, &before, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["four.m4a", "one.flac"],
        "the evening before the boundary is before it, and the half hour after \
         it is not — nothing in the tree or the data carries a zone"
    );

    let after = rule(
        MatchableField::LastPlayed,
        Comparison::After,
        boundary(midnight),
    );
    assert_rule_membership(
        &store,
        &tracks,
        &after,
        None,
        &clock,
        "after local midnight",
    );
    assert_eq!(
        rule_window(&store, &after, None)
            .iter()
            .map(rule_name)
            .collect::<Vec<_>>(),
        vec!["aged.mp3", "recent.mp3", "two.wav"],
        "the boundary's own second belongs to neither side, and every Track with no \
         history answers false: the two rows stamped weeks after the boundary are \
         `aged` and `recent`, whose history is measured back from the read"
    );

    // Negated, the same boundary takes in every Track that has no history at
    // all: absent means the test is false, and `is not` inverts that.
    let not_after = rule_not(
        MatchableField::LastPlayed,
        Comparison::After,
        boundary(midnight),
    );
    assert_rule_membership(
        &store,
        &tracks,
        &not_after,
        None,
        &clock,
        "not after midnight",
    );
    assert_eq!(
        rule_total(&store, &not_after, None),
        12,
        "fifteen rows, three of them after the boundary"
    );
}

/// A `RuleTree` naming a field id this build does not know.
fn unreadable_field(field_id: &str) -> RuleTree {
    RuleTree::Rule {
        field_id: field_id.to_string(),
        operator_id: "is".to_string(),
        negated: false,
        value: needle("whatever"),
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one refusal case per unreadable shape"
)]
fn test_an_unreadable_tree_is_refused_with_a_reason_and_never_as_an_empty_page() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, tracks) = seeded_rules_store(&db_path);
    let clock = rule_clock(&store);

    // `smart_rules.field_id` CHECKs the ids this build writes, and nothing
    // else: a row written by a newer riff still reads back. `MatchableField`
    // is closed, so the row cannot be coerced — the refusal is the answer, and
    // it arrives before any SQL is produced.
    let cases = vec![
        (
            "a field id this build cannot read",
            unreadable_field("bpm"),
            "bpm",
        ),
        (
            "an operator id this build cannot read",
            RuleTree::Rule {
                field_id: "title".to_string(),
                operator_id: "starts_with".to_string(),
                negated: false,
                value: needle("x"),
            },
            "starts_with",
        ),
        (
            "a test its field does not permit, which the schema allows",
            rule(MatchableField::Genre, Comparison::GreaterThan, number(3)),
            "greater_than",
        ),
        (
            "a value of the wrong kind for the field",
            rule(MatchableField::Year, Comparison::EqualTo, needle("1975")),
            "year",
        ),
        (
            "a day count where a day-and-a-time is what the field takes",
            rule(MatchableField::LastPlayed, Comparison::Before, days(7)),
            "last_played",
        ),
        (
            "a Rule with no value where one is needed",
            RuleTree::Rule {
                field_id: "title".to_string(),
                operator_id: "is".to_string(),
                negated: false,
                value: None,
            },
            "title",
        ),
        (
            "a combine word this build cannot read",
            RuleTree::Box {
                combine: "none".to_string(),
                children: vec![rule(MatchableField::Title, Comparison::Is, needle("x"))],
            },
            "none",
        ),
        (
            "a Box holding no Rule at all",
            all_of(Vec::new()),
            "no Rule",
        ),
        (
            "an empty Box buried inside a tree that is not empty",
            all_of(vec![
                rule(MatchableField::Year, Comparison::EqualTo, number(1975)),
                any_of(Vec::new()),
            ]),
            "no Rule",
        ),
    ];

    // A value sitting on a row whose test takes none is deliberately NOT here:
    // see the tolerance asserted after these cases.
    for (why, tree, needle_text) in cases {
        let answer = store.custom_smart_playlist_page(&tree, TrackListOrder::PathAsc, None, 0, 50);
        let error = match answer {
            Ok(page) => panic!(
                "{why}: a tree this build cannot read must be refused, not answered \
                 ({} rows, total {})",
                page.rows().len(),
                page.total()
            ),
            Err(error) => error,
        };
        let reason = error.to_string();
        assert!(
            reason.contains("cannot match this Custom Smart Playlist"),
            "{why}: the refusal names the surface it failed on, so a listing that \
             cannot be computed is distinguishable from one that matched nothing: {reason}"
        );
        assert!(
            reason.contains(needle_text),
            "{why}: the reason carries the offender's own id: {reason}"
        );
        assert!(
            !rules_reference::can_read(&tree, &clock),
            "{why}: the independent walk refuses the same tree, so the store is not \
             refusing alone by accident"
        );
    }

    // The one tolerance: a value sitting on a row whose test takes none is the
    // save path's mistake, and absence is the column's answer either way.
    let spare = RuleTree::Rule {
        field_id: "genre".to_string(),
        operator_id: "is_missing".to_string(),
        negated: false,
        value: needle("ignored"),
    };
    assert_rule_membership(
        &store,
        &tracks,
        &spare,
        None,
        &clock,
        "a spare value on a valueless test",
    );
    assert_eq!(
        rule_window(&store, &spare, None),
        rule_window(
            &store,
            &rule(MatchableField::Genre, Comparison::IsMissing, None),
            None
        ),
        "the spare value changes nothing: absence is decided by the column"
    );
}

#[test]
fn test_a_tree_wider_than_the_engine_can_bind_is_refused_with_its_own_size() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("riff.sqlite3");
    let (store, _tracks) = seeded_rules_store(&db_path);

    // Rules are uncapped, so a Box can genuinely outgrow SQLite's ceiling on
    // bound values (~32,766). That is a store error like any other: it carries
    // a reason, and no path renders it as an empty list. The compiler neither
    // chunks nor splits a predicate to dodge it — half a predicate answers a
    // different question than the one the listener wrote.
    let wide = 33_000;
    let arms = (0..wide)
        .map(|_| rule(MatchableField::Title, Comparison::Is, needle("x")))
        .collect();
    let tree = any_of(arms);
    let error = store
        .custom_smart_playlist_page(&tree, TrackListOrder::PathAsc, None, 0, 50)
        .expect_err("a tree past the binding ceiling is refused, never answered");
    let reason = error.to_string();
    assert!(
        reason.contains("cannot match this Custom Smart Playlist")
            && reason.contains("32766")
            && reason.contains("33000"),
        "the refusal names the tree's own width and the ceiling it passed: {reason}"
    );

    // Just under the ceiling the same shape answers: the limit is the engine's,
    // not a cap this build invents.
    let boxes = 30;
    let arms = 500;
    let page = store
        .custom_smart_playlist_page(
            &all_of(
                (0..boxes)
                    .map(|box_index| {
                        any_of(
                            (0..arms)
                                .map(|arm| {
                                    rule(
                                        MatchableField::Title,
                                        Comparison::Is,
                                        needle(&format!("no such title {box_index}-{arm}")),
                                    )
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            ),
            TrackListOrder::PathAsc,
            None,
            0,
            50,
        )
        .expect("a wide tree under the ceiling is answered");
    assert_eq!(
        (page.total(), page.rows().len()),
        (0, 0),
        "and it matches nothing: {} parameters, under the ceiling",
        boxes * arms
    );

    // Depth is the engine's other ceiling, and this build does not count
    // nesting levels to pre-empt it: a deep tree is prepared, and SQLite says
    // no in its own words, which the read surfaces as a reason rather than a
    // silent empty listing.
    // Depth is the engine's other ceiling, and this build does not count
    // nesting levels to pre-empt it. Four hundred Box levels is a tree this
    // store answers; eight hundred is the bundled parser saying no in its own
    // words, which the read carries out as a reason rather than swallowing it
    // or rendering it as an empty listing.
    let mut deep = rule(MatchableField::Year, Comparison::EqualTo, number(1975));
    for _ in 0..400 {
        deep = all_of(vec![deep]);
    }
    let answered = store
        .custom_smart_playlist_page(&deep, TrackListOrder::PathAsc, None, 0, 50)
        .expect("four hundred levels is a tree, not a failure");
    assert_eq!(answered.total(), 3, "and it answers the same rows");

    let mut too_deep = rule(MatchableField::Year, Comparison::EqualTo, number(1975));
    for _ in 0..800 {
        too_deep = all_of(vec![too_deep]);
    }
    let error = store
        .custom_smart_playlist_page(&too_deep, TrackListOrder::PathAsc, None, 0, 50)
        .expect_err("a tree past the engine's own expression depth is refused");
    let reason = error.to_string();
    assert!(
        reason.contains("failed to read a listing page") && reason.len() > 40,
        "the engine's reason is carried out of the read, not swallowed: {reason}"
    );
}

// --- Bounded value-picker reads over the columns a Rule tests -------------
//
// smart-playlist-rules ticket 04. A Custom Smart Playlist's value picker must
// offer only values the column the Rule actually tests can contain, so these
// reads are bounded `DISTINCT` reads over `tracks` — never over the `albums` /
// `artists` entity tables the browse listings use. The self-contained module
// keeps the whole ticket's suite in one contiguous append.
mod smart_picker_reads {
    use super::*;

    /// A `tracks` row with every picker-relevant tag under control: the four
    /// pickable text columns plus the two numeric ones. `album` and
    /// `album_artist` stay `Option` here — the fallback the scan bakes into
    /// the *entity* keys is exactly what these tests are pinning the picker
    /// reads against, so the fixture must be able to leave them absent.
    fn picker_track(
        path: &str,
        artist: Option<&str>,
        album_artist: Option<&str>,
        album: Option<&str>,
        genre: Option<&str>,
        sample_rate: Option<u32>,
        channels: Option<u16>,
    ) -> Track {
        Track::fresh_scan(
            TrackId::from_path(std::path::Path::new(path)),
            PathBuf::from(path),
            TrackMetadata {
                title: Some(format!("Track at {path}")),
                artist: artist.map(str::to_string),
                album: album.map(str::to_string),
                album_artist: album_artist.map(str::to_string),
                genre: genre.map(str::to_string),
                ..TrackMetadata::default()
            },
            Some(Duration::from_mins(3)),
            sample_rate,
            channels,
            None,
        )
    }

    /// The shared fixture library: every picker-relevant shape the feature
    /// cares about in one seed — a `;`-separated genre tag, a `;`-separated
    /// tag with no space after the delimiter, a segment padded with spaces,
    /// an empty-string genre, a `NULL` genre, an absent artist tag beside a
    /// track literally tagged `Unknown Artist`, an absent album tag beside one
    /// literally tagged `Unknown Album`, a repeated value, and absent numeric
    /// facts.
    fn seeded_picker_store() -> (tempfile::TempDir, SqliteStore) {
        let (dir, mut store, _changes_rx) = open_store();
        let tracks = vec![
            picker_track(
                "/lib/1.flac",
                Some("Aphex Twin"),
                Some("Aphex Twin"),
                Some("Selected"),
                Some("Ambient"),
                Some(44_100),
                Some(2),
            ),
            picker_track(
                "/lib/2.mp3",
                Some("Unknown Artist"),
                None,
                Some("Selected"),
                Some("Ambient; Techno"),
                Some(44_100),
                Some(1),
            ),
            picker_track(
                "/lib/3.mp3",
                None,
                None,
                None,
                Some(""),
                Some(48_000),
                Some(2),
            ),
            picker_track(
                "/lib/4.wav",
                Some("Björk"),
                Some("Björk"),
                Some("Homogenic"),
                Some("Art Rock;Progressive Rock"),
                None,
                None,
            ),
            picker_track(
                "/lib/5.ogg",
                Some("Aphex Twin"),
                Some("Warp Artists"),
                Some("Selected"),
                Some("  Techno "),
                Some(48_000),
                Some(6),
            ),
            picker_track(
                "/lib/6.m4a",
                Some("Unknown Artist"),
                Some("Unknown Artist"),
                Some("Unknown Album"),
                Some("Techno"),
                Some(96_000),
                Some(2),
            ),
        ];
        store.apply_scan_batch(&tracks).expect("fixtures apply");
        (dir, store)
    }

    /// The genre picker offers the library's distinct genre *entries*, not its
    /// distinct genre *strings*: a Rule tests genre as one whole
    /// `;`-separated segment, so `"Ambient; Techno"` can never be a value it
    /// matches and must never be offered. Space padding is trimmed away for
    /// the same reason — the picked value has to be one the Rule can answer.
    /// Empty and absent tags contribute nothing.
    #[test]
    fn genre_picker_offers_the_distinct_genre_entries_without_empty_or_absent_tags() {
        let (_dir, store) = seeded_picker_store();

        let offered = store.picker_texts("genre", 50).expect("genre picker read");

        assert_eq!(
            offered,
            vec![
                "Ambient".to_string(),
                "Art Rock".to_string(),
                "Progressive Rock".to_string(),
                "Techno".to_string(),
            ],
            "the six seeded genre strings collapse to four distinct entries"
        );
    }

    /// The artist, album-artist and album pickers answer `DISTINCT` over the
    /// raw `tracks` column, so each offers exactly the tag values the Library
    /// holds: a repeated tag appears once, an absent tag appears not at all,
    /// and the order is the ascending byte-wise one the genre picker uses.
    #[test]
    fn artist_album_artist_and_album_pickers_offer_the_distinct_stored_tags() {
        let (_dir, store) = seeded_picker_store();

        assert_eq!(
            store.picker_texts("artist", 50).expect("artist read"),
            vec![
                "Aphex Twin".to_string(),
                "Bj\u{f6}rk".to_string(),
                "Unknown Artist".to_string(),
            ],
            "the two Aphex Twin rows collapse to one entry and the absent tag on              /lib/3.mp3 contributes nothing"
        );
        assert_eq!(
            store
                .picker_texts("album_artist", 50)
                .expect("album artist read"),
            vec![
                "Aphex Twin".to_string(),
                "Bj\u{f6}rk".to_string(),
                "Unknown Artist".to_string(),
                "Warp Artists".to_string(),
            ],
            "the two rows with no album-artist tag contribute nothing, even though              their entity keys fall back"
        );
        assert_eq!(
            store.picker_texts("album", 50).expect("album read"),
            vec![
                "Homogenic".to_string(),
                "Selected".to_string(),
                "Unknown Album".to_string(),
            ],
            "Unknown Album is here only because /lib/6.m4a is literally tagged so;              the untagged /lib/3.mp3 is absent from the list"
        );
    }

    /// `Unknown Artist` is unofferable by construction, not filtered out by a
    /// name check. A store whose only "unknown" artist comes from an absent
    /// tag has the fallback in its `artists` entity table — that is what the
    /// Artists listing reads, because the scan keys entities on
    /// `display_album_artist()` — while `tracks.artist` is `NULL` and the
    /// picker therefore answers without it. The listing assertion beside it is
    /// what makes the picker assertion mean something: without a fallback to
    /// refuse, an empty picker would pass for the wrong reason.
    #[test]
    fn an_absent_artist_tag_never_offers_the_display_fallback_the_artists_listing_shows() {
        let (_dir, mut store, _changes_rx) = open_store();
        store
            .apply_scan_batch(&[
                picker_track(
                    "/only/untagged.flac",
                    None,
                    None,
                    Some("An Album"),
                    Some("Techno"),
                    Some(44_100),
                    Some(2),
                ),
                picker_track(
                    "/only/tagged.flac",
                    Some("Bj\u{f6}rk"),
                    Some("Bj\u{f6}rk"),
                    Some("Homogenic"),
                    Some("Art Rock"),
                    Some(48_000),
                    Some(1),
                ),
            ])
            .expect("fixtures apply");

        assert_eq!(
            store.picker_texts("artist", 50).expect("artist read"),
            vec!["Bj\u{f6}rk".to_string()],
            "the picker reads tracks.artist, which the absent tag leaves NULL"
        );
        assert_eq!(
            store
                .picker_texts("album_artist", 50)
                .expect("album artist read"),
            vec!["Bj\u{f6}rk".to_string()],
            "and the same holds for the album-artist column the fallback derives from"
        );
        let listed: Vec<String> = store
            .all_artists()
            .expect("artists listing reads")
            .into_iter()
            .map(|artist| artist.name)
            .collect();
        assert_eq!(
            listed,
            vec!["Bj\u{f6}rk".to_string(), "Unknown Artist".to_string()],
            "the listing really does show the fallback the picker refuses —              asking for those tracks is `is missing`, not a pick"
        );
    }

    /// The refusal is about the fallback, not the string: a track genuinely
    /// tagged `Unknown Artist` is a value its column can contain, so a Rule
    /// can match it and the picker must offer it — once, beside the absent
    /// tag's silence.
    #[test]
    fn a_literal_unknown_artist_tag_is_offerable_while_an_absent_one_is_not() {
        let (_dir, mut store, _changes_rx) = open_store();
        store
            .apply_scan_batch(&[
                picker_track(
                    "/literal/absent.mp3",
                    None,
                    None,
                    Some("An Album"),
                    Some("Techno"),
                    Some(44_100),
                    Some(2),
                ),
                picker_track(
                    "/literal/tagged.mp3",
                    Some("Unknown Artist"),
                    Some("Unknown Artist"),
                    Some("An Album"),
                    Some("Techno"),
                    Some(44_100),
                    Some(2),
                ),
            ])
            .expect("fixtures apply");

        assert_eq!(
            store.picker_texts("artist", 50).expect("artist read"),
            vec!["Unknown Artist".to_string()],
            "exactly one entry, and it is the literal tag"
        );
        assert_eq!(
            store
                .picker_texts("album_artist", 50)
                .expect("album artist read"),
            vec!["Unknown Artist".to_string()],
            "the album-artist picker agrees, and no second fallback entry appears"
        );
    }

    /// A field id the store does not catalogue as a pickable text field —
    /// because it is unknown, because it names a numeric column, or because
    /// it names a text column the catalog offers no picker for — is refused
    /// before any SQL is produced, rather than answering as an empty list a
    /// caller could not tell apart from an empty Library.
    #[test]
    fn an_id_with_no_pickable_text_column_is_refused_rather_than_answered_empty() {
        let (_dir, store) = seeded_picker_store();

        for field_id in ["not-a-field", "", "sample_rate", "title", "composer"] {
            let outcome = store.picker_texts(field_id, 50);
            assert!(
                matches!(outcome, Err(StoreError::InvalidOperation(_))),
                "{field_id} must be refused, got {outcome:?}"
            );
        }
        for field_id in ["genre", "artist", "album", "album_artist"] {
            let outcome = store.picker_numbers(field_id, 50);
            assert!(
                matches!(outcome, Err(StoreError::InvalidOperation(_))),
                "{field_id} is not a numeric picker field, got {outcome:?}"
            );
        }
    }

    /// The store's column table and the field catalog must not drift, in
    /// either direction. Every field the catalog marks pickable has to resolve
    /// to a column, or its picker would answer nothing; and the fields the
    /// catalog gives a typed control for have to stay unreachable from a
    /// picker. The second half is the one that matters over time: the column
    /// table grows with every Rule the compiler learns to test, and a Title or
    /// Composer row landing there must not widen what a listener can pick.
    #[test]
    fn the_column_table_resolves_exactly_the_catalogs_pickable_fields() {
        use riff_infra::store::{field_column, picker_column};
        use riff_persistence::field_catalog::{ValueType, fields};

        let mut pickable = 0;
        for spec in fields() {
            let id = spec.field().stored_id();
            if !spec.is_pickable() {
                assert!(
                    picker_column(id, ValueType::Text).is_none()
                        && picker_column(id, ValueType::Number).is_none(),
                    "{id} is not pickable but a picker read reaches it"
                );
                continue;
            }
            pickable += 1;
            let expected = match spec.value_type() {
                ValueType::Text => ValueType::Text,
                ValueType::Number => ValueType::Number,
                other => panic!("{id} is pickable as a {other:?}, which has no picker read"),
            };
            let resolved = picker_column(id, expected)
                .unwrap_or_else(|| panic!("{id} is pickable but names no catalogued column"));
            assert_eq!(
                Some(resolved),
                field_column(spec.field()),
                "{id} resolves through the picker gate to a different column"
            );
        }
        assert_eq!(
            pickable, 6,
            "four text fields (genre, artist, album artist, album) and two              numeric ones (sample rate, channels)"
        );
    }

    /// The two numeric pickers read `DISTINCT` over `tracks.sample_rate` and
    /// `tracks.channels`, ascending: a repeated rate collapses to one entry,
    /// an unmeasured track contributes nothing, and — unlike the text pickers
    /// — a stored `0` would be a present value and belong in the answer. The
    /// answer is `i64` because that is the column's own SQL affinity; the
    /// Rule's stored value is text spelling the same number.
    #[test]
    fn sample_rate_and_channel_pickers_offer_the_distinct_measured_values() {
        let (_dir, store) = seeded_picker_store();

        assert_eq!(
            store.picker_numbers("sample_rate", 50).expect("rate read"),
            vec![44_100, 48_000, 96_000],
            "the two rates repeat across six tracks and /lib/4.wav is unmeasured"
        );
        assert_eq!(
            store.picker_numbers("channels", 50).expect("channels read"),
            vec![1, 2, 6],
            "channel counts collapse the same way, with the absent one absent"
        );
    }

    /// A library bigger than the bound is the point of the bound: the store
    /// answers exactly `limit` values and no more, so a 20,000-track Library
    /// cannot fill a picker pane. Seeds `count` tracks each carrying its own
    /// genre, artist, album, rate and channel count, then pins the truncation
    /// for both shapes — text and numeric, plain column and genre-set.
    #[test]
    fn a_library_larger_than_the_bound_answers_exactly_the_bound() {
        const SEEDED: usize = 40;
        const BOUND: usize = 8;
        let (_dir, mut store, _changes_rx) = open_store();
        let tracks: Vec<Track> = (0..SEEDED)
            .map(|index| {
                let tag = format!("value {index:02}");
                picker_track(
                    &format!("/bounded/{index:02}.flac"),
                    Some(&tag),
                    Some(&tag),
                    Some(&tag),
                    // Two segments per tag, so the genre read has twice the
                    // distinct entries and still has to honour the bound.
                    Some(&format!("{tag}; extra {index:02}")),
                    Some(8_000 + u32::try_from(index).expect("small index") * 100),
                    Some(u16::try_from(index % 8 + 1).expect("small channel")),
                )
            })
            .collect();
        store.apply_scan_batch(&tracks).expect("fixtures apply");

        let texts = store
            .picker_texts("artist", BOUND)
            .expect("bounded artist read");
        assert_eq!(
            texts.len(),
            BOUND,
            "a 40-value library bounded at 8 answers 8, not 40 and not 9"
        );
        assert_eq!(
            texts,
            (0..BOUND)
                .map(|index| format!("value {index:02}"))
                .collect::<Vec<String>>(),
            "the bounded answer is the deterministic ascending prefix of the full one"
        );
        let genres = store
            .picker_texts("genre", BOUND)
            .expect("bounded genre read");
        assert_eq!(
            genres.len(),
            BOUND,
            "the genre read bounds its split entries, not its stored strings"
        );
        assert_eq!(
            genres,
            vec![
                "extra 00".to_string(),
                "extra 01".to_string(),
                "extra 02".to_string(),
                "extra 03".to_string(),
                "extra 04".to_string(),
                "extra 05".to_string(),
                "extra 06".to_string(),
                "extra 07".to_string(),
            ],
            "entries are ordered and truncated as entries: `value NN` sorts after              `extra NN` byte-wise, so no `value` entry can reach a bound of 8"
        );
        let rates = store
            .picker_numbers("sample_rate", BOUND)
            .expect("bounded rate read");
        assert_eq!(
            rates,
            (0..BOUND)
                .map(|index| 8_000 + i64::try_from(index).expect("small bound") * 100)
                .collect::<Vec<i64>>(),
            "the numeric read bounds the same way, ascending from the smallest value"
        );

        // The bound is the store's, and it is not a hint: asking for more than
        // exists answers every value, and asking for none answers nothing.
        assert_eq!(
            store
                .picker_numbers("channels", SEEDED + 10)
                .expect("wide read")
                .len(),
            8,
            "eight distinct channel counts is the whole answer once the library has fewer"
        );
        assert_eq!(
            store.picker_texts("artist", 0).expect("zero bound"),
            [] as [String; 0]
        );
        assert_eq!(
            store.picker_numbers("channels", 0).expect("zero bound"),
            [] as [i64; 0]
        );
    }
}
