//! The `SQLite` `Application Store`.
//!
//! riff's single authoritative persistent state lives in one embedded `SQLite`
//! database. This module owns opening the database file, configuring the
//! connection for durability, and running the embedded, checksummed migration
//! set. Open or migrate failures are fatal startup errors surfaced as clear
//! [`AppError`]s rather than silent fallbacks.

use crate::app::errors::AppError;
use crate::app::library_manager::LOST_GEMS_THRESHOLD;
use crate::app::state::{ScalarSettings, WatchState};
use crate::app::store::{
    LibraryCollection, LibraryMutationStore, LibraryQueryStore, PlaylistStore, Settings,
    SettingsStore, StoreMigrations,
};
use crate::app::MutexExt;
use crate::domain::{
    Album, Artist, Playlist, PlaylistId, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A migration is an ordered schema step with a stable identity and content
/// checksum so accidental edits are detected instead of silently applied.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

/// SHA-256 of each migration's `sql` bytes, computed once at compile time.
static MIGRATION_CHECKSUMS: &[(&str, &str)] = &[
    (
        "001_initial_schema",
        "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
    ),
    (
        "002_settings_typed_tables",
        "8417f823d5bcbf7ddbbf8d7a70764a09a36735f94f5c7046b9c51c31379054f7",
    ),
    (
        "003_playlists",
        "8d7aa79437cb4e297ccb3c0b2b7602fa2573c9bbd3ddeeb7b00c10cf321e5983",
    ),
    (
        "004_library_collection",
        "15b0d88e8583d3744ac23193b63a387784053bc42f885c53cdb93ff8321bc778",
    ),
];

/// Embedded, ordered, checksummed migrations. Append-only once shipped:
/// editing an entry (or its checksum) makes already-migrated stores fail to
/// open with a clear error instead of silently diverging.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "001_initial_schema",
        sql: "CREATE TABLE IF NOT EXISTS store_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
          );",
    },
    Migration {
        version: 2,
        name: "002_settings_typed_tables",
        // Typed Settings tables: a single-row scalar table plus explicit
        // library-path and watch-state tables (no opaque blobs). The scalar
        // row is seeded here so reads never have to special-case "missing".
        sql: "CREATE TABLE app_settings (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          volume REAL,
          advanced_mode INTEGER NOT NULL DEFAULT 0 CHECK (advanced_mode IN (0, 1)),
          high_contrast INTEGER NOT NULL DEFAULT 0 CHECK (high_contrast IN (0, 1)),
          replaygain_enabled INTEGER NOT NULL DEFAULT 0 CHECK (replaygain_enabled IN (0, 1))
        );
        INSERT INTO app_settings (id) VALUES (1);

        CREATE TABLE library_paths (
          path TEXT PRIMARY KEY
        );

        CREATE TABLE watch_states (
          path TEXT PRIMARY KEY,
          state TEXT NOT NULL CHECK (state IN ('disabled', 'enabled', 'warning')),
          warning_message TEXT
        );",
    },
    Migration {
        version: 3,
        name: "003_playlists",
        // Playlists are user data in the Application Store. Entries carry
        // NO enforced link to tracks: dangling references are valid product
        // behavior validated at read time. Deleting a playlist cascades to
        // its entries.
        sql: "CREATE TABLE playlists (
          id TEXT PRIMARY KEY,
          name TEXT NOT NULL,
          created_at INTEGER NOT NULL
        );

        CREATE TABLE playlist_entries (
          playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
          position INTEGER NOT NULL,
          track_id TEXT NOT NULL,
          PRIMARY KEY (playlist_id, position)
        );",
    },
    Migration {
        version: 4,
        name: "004_library_collection",
        // The Library collection becomes store-resident (ticket 05). Strict
        // foreign keys chain tracks → albums → artists; album identity is
        // `(album artist, title)`. Raw nullable metadata columns preserve
        // exact domain round-trips while the *_key columns carry the resolved
        // display fallbacks the FK chain and grouping need. `search_text` is
        // derived Rust-lowercased at write time for exact substring-search
        // parity with the former in-memory implementation.
        sql: "CREATE TABLE artists (
          name TEXT PRIMARY KEY
        );

        CREATE TABLE albums (
          album_artist TEXT NOT NULL,
          title TEXT NOT NULL,
          year INTEGER,
          genre TEXT,
          PRIMARY KEY (album_artist, title),
          FOREIGN KEY (album_artist) REFERENCES artists(name)
        );

        CREATE TABLE tracks (
          path TEXT PRIMARY KEY,
          title TEXT,
          artist TEXT,
          album TEXT,
          album_artist TEXT,
          track_number INTEGER,
          disc_number INTEGER,
          genre TEXT,
          year INTEGER,
          composer TEXT,
          comment TEXT,
          replaygain_track_gain REAL,
          replaygain_track_peak REAL,
          duration_nanos INTEGER,
          sample_rate INTEGER,
          channels INTEGER,
          play_count INTEGER NOT NULL DEFAULT 0 CHECK (play_count >= 0),
          last_played_nanos INTEGER,
          date_added_nanos INTEGER,
          search_text TEXT NOT NULL,
          album_artist_key TEXT NOT NULL,
          album_title_key TEXT NOT NULL,
          FOREIGN KEY (album_artist_key, album_title_key)
            REFERENCES albums(album_artist, title)
        );",
    },
];

/// Location of the Application Store database file: `riff.sqlite3` in the
/// data-local directory, mirroring where the legacy JSON files lived.
///
/// # Errors
/// Returns an error when the platform provides no data-local directory;
/// callers treat this as a fatal startup condition.
pub fn default_store_path() -> Result<std::path::PathBuf, AppError> {
    directories::ProjectDirs::from("", "", "riff")
        .map(|dirs| dirs.data_local_dir().join("riff.sqlite3"))
        .ok_or_else(|| {
            AppError::InvalidOperation(
                "no data-local directory is available on this platform".to_string(),
            )
        })
}

/// A Unix-nanosecond timestamp used to suffix corrupted store files that are
/// renamed aside, so recovery tools can tell recovery attempts apart.
fn unix_nanoseconds() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

/// Rename `path` (if it exists) beside itself with a Unix-nanosecond suffix.
/// Missing files are not an error; failures surface as [`AppError`].
fn rename_aside(path: &std::path::Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    let mut suffixed = path.as_os_str().to_owned();
    suffixed.push(format!(".{}", unix_nanoseconds()));
    std::fs::rename(path, &suffixed).map_err(|e| {
        AppError::InvalidOperation(format!(
            "failed to set aside corrupted store file {}: {e}",
            path.display()
        ))
    })
}

/// The shared `SQLite` connection backing the `Application Store`.
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Open (creating it when missing) the store at `path`, configure the
    /// connection for durability, and apply every pending migration inside
    /// one transaction per migration.
    pub fn open_and_migrate(path: &std::path::Path) -> Result<Self, AppError> {
        match Self::try_open_and_migrate(path) {
            Ok(store) => Ok(store),
            Err(failure) => Self::recover_from_failure(path, failure),
        }
    }

    /// Open the store for writing, configure it, and migrate it. Called after
    /// the read-only integrity probe passed (or the file was absent).
    fn open_writable_and_migrate(path: &std::path::Path) -> Result<Self, AppError> {
        let conn = Connection::open(path).map_err(|e| {
            AppError::InvalidOperation(format!(
                "failed to open Application Store at {}: {e}",
                path.display()
            ))
        })?;
        Self::configure_and_migrate(conn)
    }

    /// Best-effort open without recovery; the error carries the exact stage
    /// (open, integrity check, or migration) that failed.
    fn try_open_and_migrate(path: &std::path::Path) -> Result<Self, AppError> {
        // WAL recovery requires write access to the `-shm`/`-wal` siblings;
        // a corrupt database must not get the chance to trigger SQLite's own
        // recovery before we can set the broken files aside, so the first
        // connection opens in read-only mode.
        let read_only = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
        match Connection::open_with_flags(path, read_only) {
            Ok(probe) => {
                // The file exists and is readable: run the integrity check on
                // this probe connection so corruption is detected BEFORE any
                // connection attempts to write or recover the database.
                let check_result: Result<String, _> =
                    probe.query_row("PRAGMA quick_check", [], |row| row.get(0));
                drop(probe);
                if let Ok(result) = check_result {
                    if result == "ok" {
                        // Healthy store: proceed with the real (writable)
                        // connection and normal migrations.
                        return Self::open_writable_and_migrate(path);
                    }
                    return Err(AppError::InvalidOperation(format!(
                        "Application Store at {} failed integrity check: {result}",
                        path.display()
                    )));
                }
                // quick_check itself errored (e.g. unreadable schema):
                // treat as corrupt and fail through the recovery path.
                Err(AppError::InvalidOperation(format!(
                    "Application Store at {} failed integrity check (quick_check error)",
                    path.display()
                )))
            }
            Err(open_err) if path.exists() => Err(AppError::InvalidOperation(format!(
                "Application Store at {} failed integrity check: {open_err}",
                path.display()
            ))),
            Err(_missing_file) => {
                // A missing file is a normal fresh start; any other state is
                // handled by the arms above.
                Self::open_writable_and_migrate(path)
            }
        }
    }

    /// Automatic corruption recovery: when opening, checking, or migrating
    /// the store fails, the database file and its `-wal`/`-shm` siblings are
    /// renamed aside (Unix-nanosecond suffixed, preserved for recovery tools)
    /// and a fresh store is created. Only a failure of the recovery itself is
    /// a fatal startup error.
    fn recover_from_failure(path: &std::path::Path, failure: AppError) -> Result<Self, AppError> {
        tracing::warn!(
            "Application Store failed at {}: attempting automatic recovery ({failure})",
            path.display()
        );
        for suffix in ["-wal", "-shm", ""] {
            let mut sibling = path.as_os_str().to_owned();
            sibling.push(suffix);
            let existed = match std::fs::symlink_metadata(&sibling) {
                Ok(meta) => meta.is_file(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => {
                    return Err(AppError::InvalidOperation(format!(
                        "fatal: failed to inspect corrupted store file {}: {e}",
                        sibling.to_string_lossy()
                    )))
                }
            };
            if existed {
                rename_aside(std::path::Path::new(&sibling))?;
            }
        }
        match Self::try_open_and_migrate(path) {
            Ok(store) => Ok(store),
            Err(recovery_failure) => Err(AppError::InvalidOperation(format!(
                "fatal: Application Store at {} could not be recovered after {}: {recovery_failure}",
                path.display(),
                failure
            ))),
        }
    }

    /// Apply every pending migration to the already-open store. Idempotent:
    /// applied versions are verified against their embedded checksum and
    /// skipped, pending ones are applied exactly once.
    pub fn apply_migrations(&mut self) -> Result<(), AppError> {
        Self::run_migrations(&self.conn)
    }

    /// Run `f` with the underlying connection. Used by infrastructure tests
    /// at the port boundary; later store features build on this access path.
    pub fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, AppError> {
        f(&self.conn).map_err(|e| AppError::InvalidOperation(format!("store query failed: {e}")))
    }

    fn configure_and_migrate(mut conn: Connection) -> Result<Self, AppError> {
        Self::configure_connection(&mut conn)?;
        Self::run_migrations(&conn)?;
        Ok(Self { conn })
    }

    /// Durability setup required by the spec: WAL journal mode,
    /// synchronous=NORMAL, foreign keys ON, and a short busy timeout.
    fn configure_connection(conn: &mut Connection) -> Result<(), AppError> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| AppError::InvalidOperation(format!("failed to enable WAL mode: {e}")))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| {
                AppError::InvalidOperation(format!("failed to set synchronous=NORMAL: {e}"))
            })?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| {
                AppError::InvalidOperation(format!("failed to enable foreign keys: {e}"))
            })?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| AppError::InvalidOperation(format!("failed to set busy timeout: {e}")))?;
        // Folder prefix queries match paths byte-for-byte like the former
        // in-memory `Path::starts_with` checks; SQLite's default ASCII case
        // folding for LIKE would silently widen every folder query.
        conn.pragma_update(None, "case_sensitive_like", "ON")
            .map_err(|e| {
                AppError::InvalidOperation(format!("failed to enable case-sensitive LIKE: {e}"))
            })?;
        Ok(())
    }

    /// Bring the schema up to date by applying every pending migration in
    /// order. Each migration commits atomically with its bookkeeping row;
    /// any failure rolls back that migration completely (nothing partially
    /// applies) and aborts startup.
    fn run_migrations(conn: &Connection) -> Result<(), AppError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                checksum TEXT NOT NULL,
                name TEXT NOT NULL UNIQUE,
                applied_at INTEGER NOT NULL
             );",
        )
        .map_err(|e| {
            AppError::InvalidOperation(format!("failed to prepare schema_migrations table: {e}"))
        })?;

        for migration in MIGRATIONS {
            // A migration without a recorded checksum is a programming
            // error caught before it can corrupt a user store.
            let expected_checksum = MIGRATION_CHECKSUMS
                .iter()
                .find(|(name, _)| *name == migration.name)
                .map_or_else(
                    || unreachable!("every migration must have a recorded checksum"),
                    |(_, checksum)| *checksum,
                );
            let already_applied: Option<String> = conn
                .query_row(
                    "SELECT checksum FROM schema_migrations WHERE version = ?1",
                    [migration.version],
                    |row| row.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
                .map_err(|e| {
                    AppError::InvalidOperation(format!(
                        "failed to read migration state for version {}: {e}",
                        migration.version
                    ))
                })?;
            if let Some(recorded) = already_applied {
                if recorded != expected_checksum {
                    return Err(AppError::InvalidOperation(format!(
                        "migration {} ({}) has been tampered with: recorded checksum does not match the embedded migration",
                        migration.version, migration.name
                    )));
                }
                continue;
            }

            conn.execute_batch(&format!(
                "BEGIN;
                 CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    checksum TEXT NOT NULL,
                    name TEXT NOT NULL UNIQUE,
                    applied_at INTEGER NOT NULL
                 );
                 INSERT INTO schema_migrations(version, checksum, name, applied_at)
                 VALUES ({}, '{}', '{}', 0);",
                migration.version, expected_checksum, migration.name
            ))
            .and_then(|()| conn.execute_batch(migration.sql))
            .and_then(|()| conn.execute_batch("COMMIT;"))
            .map_err(|e| {
                let _ = conn.execute_batch("ROLLBACK;");
                AppError::InvalidOperation(format!(
                    "failed to apply migration {} ({}): {e}",
                    migration.version, migration.name
                ))
            })?;
        }
        Ok(())
    }
}

impl StoreMigrations for SqliteStore {
    fn open_and_migrate(&self, path: &std::path::Path) -> Result<(), AppError> {
        Self::open_and_migrate(path).map(|_| ())
    }
}

/// UTC epoch-nanosecond integer timestamps, the store's on-disk format for
/// playlist creation times (spec: nullable where unknown; playlists always
/// stamp at creation).
fn system_time_from_nanos(nanos: i64) -> SystemTime {
    let offset = std::time::Duration::from_nanos(nanos.unsigned_abs());
    if nanos >= 0 {
        UNIX_EPOCH + offset
    } else {
        UNIX_EPOCH - offset
    }
}

fn nanos_from_system_time(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
    }
}

impl PlaylistStore for SqliteStore {
    /// Load every Playlist in creation order with its entries in playlist
    /// order. Dangling Track references load unchanged — validity is decided
    /// at read time by the app layer, never by the schema.
    fn load_playlists(&self) -> Result<Vec<Playlist>, AppError> {
        self.with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, name, created_at FROM playlists ORDER BY rowid")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            let mut playlists = Vec::new();
            for row in rows {
                let (id, name, created_at) = row?;
                let mut entries = conn.prepare(
                    "SELECT track_id FROM playlist_entries
                     WHERE playlist_id = ?1 ORDER BY position",
                )?;
                let tracks = entries
                    .query_map([&id], |r| r.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                playlists.push(Playlist {
                    id: PlaylistId(id),
                    name,
                    tracks: tracks.into_iter().map(TrackId).collect(),
                    created: Some(system_time_from_nanos(created_at)),
                });
            }
            Ok(playlists)
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to load playlists: {e}")))
    }

    /// One immediate durable transaction: the playlist row plus its initial
    /// entries commit together or not at all.
    fn create_playlist(
        &mut self,
        name: &str,
        initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            match Self::create_playlist_in_tx(conn, name, initial_tracks) {
                Ok(id) => conn.execute_batch("COMMIT;").map(|()| id),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to create playlist: {e}")))
    }

    /// One immediate durable transaction: only the renamed row is written.
    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let updated = conn.execute(
                "UPDATE playlists SET name = ?2 WHERE id = ?1",
                rusqlite::params![id.0, new_name.trim()],
            );
            match updated {
                Ok(_) => conn.execute_batch("COMMIT;").map(|()| updated.unwrap_or(0)),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map(|rows| rows > 0)
        .map_err(|e| AppError::InvalidOperation(format!("failed to rename playlist: {e}")))
    }

    /// One immediate durable transaction: the playlist row goes and its
    /// entries cascade; no other playlist's data is rewritten.
    fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let deleted = conn.execute("DELETE FROM playlists WHERE id = ?1", [&id.0]);
            match deleted {
                Ok(_) => conn.execute_batch("COMMIT;").map(|()| deleted.unwrap_or(0)),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map(|rows| rows > 0)
        .map_err(|e| AppError::InvalidOperation(format!("failed to delete playlist: {e}")))
    }

    /// One immediate durable transaction: a single appended entry row, or
    /// nothing when the playlist is unknown or the entry already exists.
    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Result<bool, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let outcome = (|| -> rusqlite::Result<bool> {
                let known: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM playlists WHERE id = ?1",
                    [&id.0],
                    |row| row.get(0),
                )?;
                if known == 0 {
                    return Ok(false);
                }
                let duplicate: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM playlist_entries
                     WHERE playlist_id = ?1 AND track_id = ?2",
                    rusqlite::params![id.0, track.0],
                    |row| row.get(0),
                )?;
                if duplicate > 0 {
                    return Ok(false);
                }
                let next: i64 = conn.query_row(
                    "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_entries
                     WHERE playlist_id = ?1",
                    [&id.0],
                    |row| row.get(0),
                )?;
                conn.execute(
                    "INSERT INTO playlist_entries(playlist_id, position, track_id)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![id.0, next, track.0],
                )?;
                Ok(true)
            })();
            match outcome {
                Ok(committed) => conn.execute_batch("COMMIT;").map(|()| committed),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to add playlist entry: {e}")))
    }

    /// One immediate durable transaction removing every occurrence of the
    /// Track reference from the playlist's entries.
    fn remove_playlist_entries(
        &mut self,
        id: &PlaylistId,
        track: &TrackId,
    ) -> Result<bool, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let removed = conn.execute(
                "DELETE FROM playlist_entries WHERE playlist_id = ?1 AND track_id = ?2",
                rusqlite::params![id.0, track.0],
            );
            match removed {
                Ok(_) => conn.execute_batch("COMMIT;").map(|()| removed.unwrap_or(0)),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map(|rows| rows > 0)
        .map_err(|e| AppError::InvalidOperation(format!("failed to remove playlist entries: {e}")))
    }

    /// One immediate durable transaction rewriting the playlist's entries to
    /// exactly `ordered` (positions 0..n): the delete and every reinsert
    /// commit together or not at all, so a crash mid-reorder can never leave
    /// a truncated or duplicated entry list.
    fn reorder_playlist_entries(
        &mut self,
        id: &PlaylistId,
        ordered: &[TrackId],
    ) -> Result<bool, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let outcome = (|| -> rusqlite::Result<bool> {
                let known: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM playlists WHERE id = ?1",
                    [&id.0],
                    |row| row.get(0),
                )?;
                if known == 0 {
                    return Ok(false);
                }
                conn.execute(
                    "DELETE FROM playlist_entries WHERE playlist_id = ?1",
                    [&id.0],
                )?;
                for (position, track) in ordered.iter().enumerate() {
                    conn.execute(
                        "INSERT INTO playlist_entries(playlist_id, position, track_id)
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![
                            id.0,
                            i64::try_from(position).unwrap_or(i64::MAX),
                            track.0
                        ],
                    )?;
                }
                Ok(true)
            })();
            match outcome {
                Ok(committed) => conn.execute_batch("COMMIT;").map(|()| committed),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to reorder playlist entries: {e}")))
    }
}

impl LibraryMutationStore for SqliteStore {
    /// One immediate durable transaction per batch: parents are created
    /// first (album identity `(album artist, title)`, year/genre from the
    /// first-added track), then every track upserts by path with its play
    /// history columns excluded from the update branch, so rescans refresh
    /// metadata without ever touching history.
    fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            match Self::apply_scan_batch_in_tx(conn, tracks) {
                Ok(written) => conn.execute_batch("COMMIT;").map(|()| written),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to apply scan batch: {e}")))
    }

    /// One immediate durable transaction per finished play: a single-row
    /// update bumps `play_count` and stamps `last_played` together, so a
    /// crash right afterward cannot lose the play.
    fn record_track_played(
        &mut self,
        id: &TrackId,
        played_at: SystemTime,
    ) -> Result<bool, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let updated = conn.execute(
                "UPDATE tracks SET play_count = play_count + 1, last_played_nanos = ?1
                 WHERE path = ?2",
                rusqlite::params![nanos_from_system_time(played_at), id.0],
            );
            match updated {
                Ok(_) => conn
                    .execute_batch("COMMIT;")
                    .map(|()| updated.unwrap_or(0) > 0),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to record played track: {e}")))
    }

    /// One immediate durable transaction for a tag edit: the metadata upsert
    /// (history columns excluded) plus the album year/genre re-derivation and
    /// orphan cleanup commit together or not at all.
    fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            match Self::apply_tag_refresh_in_tx(conn, track) {
                Ok(()) => conn.execute_batch("COMMIT;"),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to apply tag refresh: {e}")))
    }

    /// One immediate durable transaction removing exactly the root's tracks,
    /// their orphaned parents, and the root's own library-path record.
    /// Playlist entries are deliberately untouched — dangling references are
    /// valid product behavior.
    fn remove_library_path(&mut self, root: &std::path::Path) -> Result<usize, AppError> {
        let root_text = root.to_string_lossy().into_owned();
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            // Byte-prefix match mirroring `Path::starts_with`: the root
            // itself (exact match) or the root followed by a path separator,
            // so "m:\music" can never swallow "m:\music2\...".
            let removed = conn.execute(
                "DELETE FROM tracks
                 WHERE path = ?1
                    OR (substr(path, 1, length(?1)) = ?1
                        AND substr(path, length(?1) + 1, 1) IN ('\\', '/'))",
                [&root_text],
            );
            let outcome = removed.and_then(|count| {
                Self::delete_orphaned_parents(conn)?;
                conn.execute("DELETE FROM library_paths WHERE path = ?1", [&root_text])?;
                Ok(count)
            });
            match outcome {
                Ok(count) => conn.execute_batch("COMMIT;").map(|()| count),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to remove library path: {e}")))
    }

    /// One immediate durable transaction: all tracks (history included),
    /// then every album and artist left without tracks via the shared
    /// orphan cleanup. Playlists and Settings tables are never touched. Any
    /// failure rolls the whole wipe back — nothing partially clears.
    fn clear_library(&mut self) -> Result<usize, AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let outcome = conn.execute("DELETE FROM tracks", []).and_then(|count| {
                Self::delete_orphaned_parents(conn)?;
                Ok(count)
            });
            match outcome {
                Ok(count) => conn.execute_batch("COMMIT;").map(|()| count),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to clear the library: {e}")))
    }
}

/// UTC epoch-nanosecond integer encoding for a `Duration` (the store's
/// on-disk format for track durations), saturating at `i64::MAX`.
fn duration_to_nanos(duration: Duration) -> i64 {
    i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
}

impl SqliteStore {
    /// Scan-batch body shared with the transaction wrapper above.
    fn apply_scan_batch_in_tx(conn: &Connection, tracks: &[Track]) -> rusqlite::Result<usize> {
        let mut written = 0;
        for track in tracks {
            // Resolved display fallbacks drive grouping and the FK chain;
            // raw optional metadata is stored alongside for exact
            // round-trips (search parity uses the raw values).
            let album_artist_key = track.metadata.display_album_artist();
            let album_title_key = track.metadata.display_album();
            conn.execute(
                "INSERT OR IGNORE INTO artists(name) VALUES (?1)",
                [&album_artist_key],
            )?;
            // OR IGNORE keeps the first-added track's year/genre derivation.
            conn.execute(
                "INSERT OR IGNORE INTO albums(album_artist, title, year, genre)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    album_artist_key,
                    album_title_key,
                    track.metadata.year.map(i64::from),
                    track.metadata.genre,
                ],
            )?;

            let search_text = track.metadata.search_text();
            let duration_nanos = track.duration.map(duration_to_nanos);
            conn.execute(
                "INSERT INTO tracks(
                    path, title, artist, album, album_artist,
                    track_number, disc_number, genre, year, composer, comment,
                    replaygain_track_gain, replaygain_track_peak,
                    duration_nanos, sample_rate, channels,
                    play_count, last_played_nanos, date_added_nanos,
                    search_text, album_artist_key, album_title_key
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13,
                    ?14, ?15, ?16,
                    0, NULL, ?17,
                    ?18, ?19, ?20
                 )
                 ON CONFLICT(path) DO UPDATE SET
                    title = ?2, artist = ?3, album = ?4, album_artist = ?5,
                    track_number = ?6, disc_number = ?7, genre = ?8, year = ?9,
                    composer = ?10, comment = ?11,
                    replaygain_track_gain = ?12, replaygain_track_peak = ?13,
                    duration_nanos = ?14, sample_rate = ?15, channels = ?16,
                    search_text = ?18, album_artist_key = ?19, album_title_key = ?20",
                rusqlite::params![
                    track.id.0,
                    track.metadata.title,
                    track.metadata.artist,
                    track.metadata.album,
                    track.metadata.album_artist,
                    track.metadata.track_number.map(i64::from),
                    track.metadata.disc_number.map(i64::from),
                    track.metadata.genre,
                    track.metadata.year.map(i64::from),
                    track.metadata.composer,
                    track.metadata.comment,
                    track.metadata.replaygain_track_gain.map(f64::from),
                    track.metadata.replaygain_track_peak.map(f64::from),
                    duration_nanos,
                    track.sample_rate.map(i64::from),
                    track.channels.map(i64::from),
                    track.date_added.map(nanos_from_system_time),
                    search_text,
                    album_artist_key,
                    album_title_key,
                ],
            )?;
            written += 1;
        }
        Ok(written)
    }

    /// Tag-refresh body shared with the transaction wrapper above: upsert the
    /// edited track (history columns excluded by the scan-batch upsert), then
    /// re-derive year/genre for every affected album from its first-added
    /// remaining track, and drop albums left empty plus artists left without
    /// albums so a track moving between albums cannot leave phantoms behind.
    fn apply_tag_refresh_in_tx(conn: &Connection, track: &Track) -> rusqlite::Result<()> {
        // Where the track lived before the edit, so the album it vacated is
        // re-derived (or cleaned up) too.
        let previous_keys: Option<(String, String)> = conn
            .query_row(
                "SELECT album_artist_key, album_title_key FROM tracks WHERE path = ?1",
                [&track.id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;

        Self::apply_scan_batch_in_tx(conn, std::slice::from_ref(track))?;

        let new_key = (
            track.metadata.display_album_artist(),
            track.metadata.display_album(),
        );
        let mut affected: Vec<(String, String)> = Vec::with_capacity(2);
        if let Some(old) = previous_keys {
            if old != new_key {
                affected.push(old);
            }
        }
        affected.push(new_key);

        for (album_artist, album_title) in affected {
            // First-added remaining track drives the derivation; tracks
            // without a date sort last so insertion order (rowid) decides.
            let derived = conn
                .query_row(
                    "SELECT year, genre FROM tracks
                     WHERE album_artist_key = ?1 AND album_title_key = ?2
                     ORDER BY COALESCE(date_added_nanos, 9223372036854775807) ASC, rowid ASC
                     LIMIT 1",
                    rusqlite::params![album_artist, album_title],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            if let Some((year, genre)) = derived {
                conn.execute(
                    "UPDATE albums SET year = ?1, genre = ?2
                     WHERE album_artist = ?3 AND title = ?4",
                    rusqlite::params![year, genre, album_artist, album_title],
                )?;
            }
        }

        Self::delete_orphaned_parents(conn)
    }

    /// Delete albums with no remaining tracks, then artists with no remaining
    /// albums. Albums are deleted first so an artist's emptiness is judged
    /// after its dead albums are gone. Must run inside an open transaction.
    fn delete_orphaned_parents(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM albums WHERE NOT EXISTS (
                SELECT 1 FROM tracks
                WHERE tracks.album_artist_key = albums.album_artist
                  AND tracks.album_title_key = albums.title
             )",
            [],
        )?;
        conn.execute(
            "DELETE FROM artists WHERE NOT EXISTS (
                SELECT 1 FROM albums WHERE albums.album_artist = artists.name
             )",
            [],
        )?;
        Ok(())
    }
}

/// UTC epoch-nanosecond decoding back into a `Duration`.
fn duration_from_nanos(nanos: i64) -> Duration {
    Duration::from_nanos(nanos.unsigned_abs())
}

/// Saturating widening used when reading integer columns back into narrow
/// domain types; values are written under CHECK constraints, so saturation
/// is unreachable in practice.
fn narrow_u32(value: Option<i64>) -> Option<u32> {
    value.map(|v| u32::try_from(v).unwrap_or(u32::MAX))
}

fn narrow_u16(value: Option<i64>) -> Option<u16> {
    value.map(|v| u16::try_from(v).unwrap_or(u16::MAX))
}

/// The track columns every read selects, in [`track_from_row`] order.
const TRACK_COLUMNS: &str = "path, title, artist, album, album_artist,
            track_number, disc_number, genre, year, composer, comment,
            replaygain_track_gain, replaygain_track_peak,
            duration_nanos, sample_rate, channels,
            play_count, last_played_nanos, date_added_nanos";

/// Escape SQL-LIKE wildcards and the escape character itself so a path
/// component matches literally under `LIKE ... ESCAPE '#'`: `%` and `_`
/// lose their wildcard meaning and a literal `#` cannot start an escape
/// sequence.
fn escape_like_pattern(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '#' => out.push_str("##"),
            '%' => out.push_str("#%"),
            '_' => out.push_str("#_"),
            _ => out.push(c),
        }
    }
    out
}

/// Component-wise folder prefix matching over stored track paths: the path
/// equals `?1` (the folder itself) or continues it with a path separator.
/// `?2`/`?3` carry the escaped folder followed by `/` resp. `\`; the ESCAPE
/// clause keeps `%`, `_`, and `#` in names literal. Combined with
/// `case_sensitive_like` this reproduces the former `Path::starts_with`
/// checks exactly — including the sibling-prefix trap (`a` never matches
/// `ab\...`).
const FOLDER_PREFIX_SQL: &str = "(path = ?1 OR path LIKE ?2 ESCAPE '#' OR path LIKE ?3 ESCAPE '#')";

/// Bind `[FOLDER_PREFIX_SQL]`'s three parameters for `folder`.
fn folder_prefix_params(folder_text: &str) -> [String; 3] {
    // Path parsing ignores trailing separators ("dir\" ≡ "dir") for
    // `starts_with`, so the query must too; a lone separator stays as-is.
    let mut trimmed = folder_text;
    while trimmed.len() > 1 {
        match trimmed.strip_suffix(['\\', '/']) {
            Some(stripped) => trimmed = stripped,
            None => break,
        }
    }
    let escaped = escape_like_pattern(trimmed);
    [
        trimmed.to_string(),
        format!("{escaped}/%"),
        format!("{escaped}\\%"),
    ]
}

/// Narrow a `REAL` column value back into the domain's `f32` tag fields.
/// The store writes these as widened `f64`s; narrowing is exact for values
/// that were `f32` to begin with.
#[allow(clippy::cast_possible_truncation)]
fn narrow_f32(value: f64) -> f32 {
    value as f32
}

/// Reconstruct a domain Track from a row selecting [`TRACK_COLUMNS`].
fn track_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    let path: String = row.get(0)?;
    Ok(Track {
        id: TrackId(path.clone()),
        file_path: PathBuf::from(path),
        metadata: TrackMetadata {
            title: row.get(1)?,
            artist: row.get(2)?,
            album: row.get(3)?,
            album_artist: row.get(4)?,
            track_number: narrow_u32(row.get(5)?),
            disc_number: narrow_u32(row.get(6)?),
            genre: row.get(7)?,
            year: narrow_u32(row.get(8)?),
            composer: row.get(9)?,
            comment: row.get(10)?,
            replaygain_track_gain: row.get::<_, Option<f64>>(11)?.map(narrow_f32),
            replaygain_track_peak: row.get::<_, Option<f64>>(12)?.map(narrow_f32),
        },
        duration: row.get::<_, Option<i64>>(13)?.map(duration_from_nanos),
        sample_rate: narrow_u32(row.get(14)?),
        channels: narrow_u16(row.get(15)?),
        play_count: narrow_u32(Some(row.get::<_, i64>(16)?)).unwrap_or(0),
        last_played: row.get::<_, Option<i64>>(17)?.map(system_time_from_nanos),
        date_added: row.get::<_, Option<i64>>(18)?.map(system_time_from_nanos),
    })
}

impl LibraryQueryStore for SqliteStore {
    /// Resolve one `Track` by its `TrackId` (its full file path).
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks WHERE path = ?1"
            ))?;
            let mut rows = stmt.query_map([&id.0], track_from_row)?;
            rows.next().transpose()
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to resolve track: {e}")))
    }

    /// One bounded window of the flat library list, path-ascending.
    fn tracks_window(&self, offset: usize, limit: usize) -> Result<Vec<Track>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks ORDER BY path ASC LIMIT ?1 OFFSET ?2"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                ],
                track_from_row,
            )?;
            rows.collect()
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to list tracks: {e}")))
    }

    fn track_count(&self) -> Result<usize, AppError> {
        self.with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to count tracks: {e}")))
    }

    /// One bounded window of case-insensitive substring matches over title,
    /// artist, album, and album artist, path-ascending. The query is
    /// lowercased in Rust so `SQLite` never applies its own case folding
    /// (non-Latin parity), and `instr()` keeps `%` and `_` literal — no LIKE
    /// wildcard semantics, exactly matching the former `str::contains`.
    fn search_window(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Track>, AppError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            let mut stmt = conn.prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks
                 WHERE instr(search_text, ?1) > 0
                 ORDER BY path ASC LIMIT ?2 OFFSET ?3"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![
                    needle,
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                ],
                track_from_row,
            )?;
            rows.collect()
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to search tracks: {e}")))
    }

    fn search_count(&self, query: &str) -> Result<usize, AppError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            conn.query_row(
                "SELECT COUNT(*) FROM tracks WHERE instr(search_text, ?1) > 0",
                [needle],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to count matches: {e}")))
    }

    /// Full collection snapshot hydrating the transitional in-memory mirror:
    /// albums keyed by the legacy `"album artist - title"` composite format,
    /// artists listing their album keys in first-added order.
    fn load_collection(&self) -> Result<LibraryCollection, AppError> {
        self.with_connection(|conn| {
            // Tracks in deterministic path order.
            let mut stmt = conn.prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks ORDER BY path ASC"
            ))?;
            let tracks: Vec<Track> = stmt
                .query_map([], track_from_row)?
                .collect::<Result<Vec<_>, _>>()?;

            // Albums in first-added order; year/genre already carry the
            // first-added-track derivation from write time.
            let mut stmt =
                conn.prepare("SELECT album_artist, title, year, genre FROM albums ORDER BY rowid")?;
            let mut albums: Vec<Album> = stmt
                .query_map([], |row| {
                    Ok(Album {
                        artist: row.get(0)?,
                        title: row.get(1)?,
                        year: narrow_u32(row.get(2)?),
                        genre: row.get(3)?,
                        tracks: Vec::new(),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            // Album membership ordered by number with missing numbers first
            // (legacy `unwrap_or(0)` sort) and a path tiebreak.
            let mut stmt = conn.prepare(
                "SELECT album_artist_key, album_title_key, path FROM tracks
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC",
            )?;
            let members = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut membership: HashMap<(String, String), Vec<TrackId>> = HashMap::new();
            for member in members {
                let (artist, title, path) = member?;
                membership
                    .entry((artist, title))
                    .or_default()
                    .push(TrackId(path));
            }
            for album in &mut albums {
                if let Some(ids) = membership.get(&(album.artist.clone(), album.title.clone())) {
                    album.tracks.clone_from(ids);
                }
            }

            // Artists A–Z with their album keys in first-added order.
            let mut album_keys: HashMap<String, Vec<String>> = HashMap::new();
            for album in &albums {
                album_keys
                    .entry(album.artist.clone())
                    .or_default()
                    .push(format!("{} - {}", album.artist, album.title));
            }
            let mut stmt = conn.prepare("SELECT name FROM artists ORDER BY name ASC")?;
            let names: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let artists = names
                .into_iter()
                .map(|name| Artist {
                    albums: album_keys.remove(&name).unwrap_or_default(),
                    name,
                })
                .collect();

            Ok(LibraryCollection {
                tracks: tracks.into_iter().map(|t| (t.id.clone(), t)).collect(),
                artists,
                albums,
            })
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to load collection: {e}")))
    }

    /// Every artist name-ascending (byte-wise, matching the former UI sort
    /// over the in-memory mirror), each carrying its album keys in canonical
    /// browsing order. Two ordered reads; grouping happens in Rust so the
    /// per-artist album order survives.
    fn all_artists(&self) -> Result<Vec<Artist>, AppError> {
        self.with_connection(|conn| {
            // Album rows arrive grouped by artist (first sort key) and in
            // canonical order within each artist: year descending with
            // missing years last, then title ascending — byte-wise, exactly
            // like the former Rust `cmp` sorts.
            let mut stmt = conn.prepare(
                "SELECT album_artist, title FROM albums
                 ORDER BY album_artist ASC, COALESCE(year, 0) DESC, title ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut keys_by_artist: HashMap<String, Vec<String>> = HashMap::new();
            for row in rows {
                let (artist, title) = row?;
                keys_by_artist
                    .entry(artist.clone())
                    .or_default()
                    .push(format!("{artist} - {title}"));
            }

            let mut stmt = conn.prepare("SELECT name FROM artists ORDER BY name ASC")?;
            let names = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(names
                .into_iter()
                .map(|name| Artist {
                    albums: keys_by_artist.remove(&name).unwrap_or_default(),
                    name,
                })
                .collect())
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to list artists: {e}")))
    }

    /// One artist's albums newest-first (missing year last) then title,
    /// each with its track ids in album-track order so an expanded artist
    /// renders without per-album queries.
    fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT title, year, genre FROM albums
                 WHERE album_artist = ?1
                 ORDER BY COALESCE(year, 0) DESC, title ASC",
            )?;
            let mut albums: Vec<Album> = stmt
                .query_map([artist], |row| {
                    Ok(Album {
                        artist: artist.to_string(),
                        title: row.get(0)?,
                        year: narrow_u32(row.get(1)?),
                        genre: row.get(2)?,
                        tracks: Vec::new(),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            // Membership arrives globally ordered by number-then-path;
            // appending per album preserves that order inside each album.
            let mut stmt = conn.prepare(
                "SELECT album_title_key, path FROM tracks
                 WHERE album_artist_key = ?1
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC",
            )?;
            let members = stmt.query_map([artist], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut membership: HashMap<String, Vec<TrackId>> = HashMap::new();
            for member in members {
                let (title, path) = member?;
                membership.entry(title).or_default().push(TrackId(path));
            }
            for album in &mut albums {
                if let Some(ids) = membership.get(&album.title) {
                    album.tracks.clone_from(ids);
                }
            }
            Ok(albums)
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to list artist albums: {e}")))
    }

    /// One album's tracks in full: track number ascending with missing
    /// numbers first (the legacy `unwrap_or(0)` slot), path tiebreak — the
    /// same ordering `load_collection` uses for album membership.
    fn album_tracks(&self, album_artist: &str, album_title: &str) -> Result<Vec<Track>, AppError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks
                 WHERE album_artist_key = ?1 AND album_title_key = ?2
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC"
            ))?;
            let rows =
                stmt.query_map(rusqlite::params![album_artist, album_title], track_from_row)?;
            rows.collect()
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to list album tracks: {e}")))
    }

    /// Escaped prefix existence check over stored track paths.
    fn folder_has_audio(&self, folder: &std::path::Path) -> Result<bool, AppError> {
        let params = folder_prefix_params(&folder.to_string_lossy());
        self.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM tracks WHERE {FOLDER_PREFIX_SQL})"),
                rusqlite::params![params[0], params[1], params[2]],
                |row| row.get(0),
            )?;
            Ok(exists > 0)
        })
        .map_err(|e| AppError::InvalidOperation(format!("folder probe failed: {e}")))
    }

    /// Escaped prefix match combined with the flat search's literal
    /// substring semantics over the derived lowercased search text.
    fn folder_has_search_match(
        &self,
        folder: &std::path::Path,
        query: &str,
    ) -> Result<bool, AppError> {
        let needle = query.to_lowercase();
        let params = folder_prefix_params(&folder.to_string_lossy());
        self.with_connection(|conn| {
            let exists: i64 = conn.query_row(
                &format!(
                    "SELECT EXISTS(
                        SELECT 1 FROM tracks
                        WHERE {FOLDER_PREFIX_SQL} AND instr(search_text, ?4) > 0
                     )"
                ),
                rusqlite::params![params[0], params[1], params[2], needle],
                |row| row.get(0),
            )?;
            Ok(exists > 0)
        })
        .map_err(|e| AppError::InvalidOperation(format!("folder search failed: {e}")))
    }

    /// Every path under `folder`, then re-sorted in Rust with the exact
    /// component-wise `Path` comparator the former in-memory tree listing
    /// used — byte-wise SQL order can differ around names that continue a
    /// sibling's name with a low byte (`a.b` vs `a\x`).
    fn track_ids_in_folder_tree(&self, folder: &std::path::Path) -> Result<Vec<TrackId>, AppError> {
        let mut paths = self.folder_paths_under(folder)?;
        paths.sort_by(|a, b| std::path::Path::new(a).cmp(std::path::Path::new(b)));
        Ok(paths.into_iter().map(TrackId).collect())
    }

    /// Tracks whose parent is exactly `folder`. Within one parent directory,
    /// filename byte order equals full-path byte order, so the SQL ordering
    /// reproduces the former number-then-filename sort exactly.
    fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, AppError> {
        let folder_text = folder.to_string_lossy().into_owned();
        let params = folder_prefix_params(&folder_text);
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks
                 WHERE {FOLDER_PREFIX_SQL}
                   AND length(path) > length(?1) + 1
                   AND instr(substr(path, length(?1) + 2), '\\') = 0
                   AND instr(substr(path, length(?1) + 2), '/') = 0
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![params[0], params[1], params[2]],
                track_from_row,
            )?;
            rows.collect()
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to list folder: {e}")))
    }

    /// Direct child directories with audio. The escaped prefix query yields
    /// every stored path two-or-more levels below `folder`; grouping into
    /// first components happens in Rust so the child set, its dedupe, and
    /// its `PathBuf` ordering replicate the former tree walk exactly. A
    /// first component counts as a directory only when something lives
    /// deeper beneath it — the structural stand-in for the former
    /// `is_dir()` stat, which excluded files sitting directly in `folder`.
    fn subdirs_with_audio(&self, folder: &std::path::Path) -> Result<Vec<PathBuf>, AppError> {
        let paths = self.folder_paths_under(folder)?;
        let mut seen = std::collections::HashSet::new();
        let mut dirs: Vec<PathBuf> = Vec::new();
        for path in paths {
            let track_path = PathBuf::from(&path);
            let Ok(relative) = track_path.strip_prefix(folder) else {
                continue;
            };
            let mut components = relative.iter();
            let Some(first_component) = components.next() else {
                continue;
            };
            if components.next().is_none() {
                // Only the file itself remains below `folder`: its name is
                // not a child directory.
                continue;
            }
            let child_dir = folder.join(first_component);
            if seen.insert(child_dir.clone()) {
                dirs.push(child_dir);
            }
        }
        dirs.sort();
        Ok(dirs)
    }

    /// Most Played: the play-count filter and primary ordering run in SQL;
    /// the display-title fallback tie-break (file-stem with underscores
    /// turned into spaces) is Rust string logic, so final ordering and the
    /// limit apply in Rust with the exact former comparator. Fetching all
    /// played rows keeps tie handling at the limit boundary faithful.
    fn smart_playlist(
        &self,
        kind: SmartPlaylistKind,
        limit: usize,
    ) -> Result<Vec<Track>, AppError> {
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        match kind {
            SmartPlaylistKind::MostPlayed => {
                let mut played: Vec<Track> = self.with_connection(|conn| {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {TRACK_COLUMNS} FROM tracks WHERE play_count > 0"
                    ))?;
                    let rows = stmt.query_map([], track_from_row)?;
                    rows.collect()
                })?;
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
                played.truncate(limit);
                Ok(played)
            }
            // Newest first by the stored first-add stamp; missing dates
            // never qualify (the mirror filtered them out entirely).
            SmartPlaylistKind::RecentlyAdded => self.with_connection(|conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {TRACK_COLUMNS} FROM tracks
                     WHERE date_added_nanos IS NOT NULL
                     ORDER BY date_added_nanos DESC, path ASC
                     LIMIT ?1"
                ))?;
                let rows = stmt.query_map([limit_i64], track_from_row)?;
                rows.collect()
            }),
            // Path-ascending unplayed list.
            SmartPlaylistKind::NeverPlayed => self.with_connection(|conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {TRACK_COLUMNS} FROM tracks
                     WHERE play_count = 0
                     ORDER BY path ASC
                     LIMIT ?1"
                ))?;
                let rows = stmt.query_map([limit_i64], track_from_row)?;
                rows.collect()
            }),
            // Longest-unheard gems (older than the threshold, or stamped in
            // the future — the mirror treated clock anomalies as "very old")
            // followed by never-played tracks in path order. The composite
            // key groups gems before unheard rows; within unheard rows every
            // timestamp is NULL so the path decides.
            SmartPlaylistKind::LostGems => {
                let now_nanos = nanos_from_system_time(SystemTime::now());
                let threshold_nanos =
                    i64::try_from(LOST_GEMS_THRESHOLD.as_nanos()).unwrap_or(i64::MAX);
                let cutoff = now_nanos.saturating_sub(threshold_nanos);
                self.with_connection(|conn| {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {TRACK_COLUMNS} FROM tracks
                         WHERE last_played_nanos IS NULL
                            OR last_played_nanos < ?1
                            OR last_played_nanos > ?2
                         ORDER BY (last_played_nanos IS NULL) ASC,
                                  last_played_nanos ASC,
                                  path ASC
                         LIMIT ?3"
                    ))?;
                    let rows = stmt.query_map(
                        rusqlite::params![cutoff, now_nanos, limit_i64],
                        track_from_row,
                    )?;
                    rows.collect()
                })
            }
        }
        .map_err(|e| AppError::InvalidOperation(format!("smart playlist query failed: {e}")))
    }
}

impl SqliteStore {
    /// Every stored track path under `folder` via [`FOLDER_PREFIX_SQL`] —
    /// including the folder's own exact path when one exists, mirroring
    /// `Path::starts_with`. Callers that must not see it skip it while
    /// grouping (`subdirs_with_audio` drops empty remainders).
    fn folder_paths_under(&self, folder: &std::path::Path) -> Result<Vec<String>, AppError> {
        let params = folder_prefix_params(&folder.to_string_lossy());
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT path FROM tracks WHERE {FOLDER_PREFIX_SQL}"
            ))?;
            let rows = stmt
                .query_map(rusqlite::params![params[0], params[1], params[2]], |row| {
                    row.get::<_, String>(0)
                })?;
            rows.collect()
        })
        .map_err(|e| AppError::InvalidOperation(format!("folder listing failed: {e}")))
    }
}

impl SqliteStore {
    /// Create-playlist body shared with the transaction wrapper above.
    /// Id generation dedupes against existing ids exactly like the former
    /// JSON-era logic so same-millisecond creation of same-named playlists
    /// cannot collide; duplicate names are allowed.
    fn create_playlist_in_tx(
        conn: &Connection,
        name: &str,
        initial_tracks: &[TrackId],
    ) -> rusqlite::Result<PlaylistId> {
        let mut id = PlaylistId::new(name);
        let mut suffix = 2;
        loop {
            let taken: i64 = conn.query_row(
                "SELECT COUNT(*) FROM playlists WHERE id = ?1",
                [&id.0],
                |row| row.get(0),
            )?;
            if taken == 0 {
                break;
            }
            id = PlaylistId(format!("{}-{suffix}", id.0));
            suffix += 1;
        }

        conn.execute(
            "INSERT INTO playlists(id, name, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![id.0, name.trim(), nanos_from_system_time(SystemTime::now()),],
        )?;

        // Initial entries: exact duplicates dropped, order preserved.
        let mut seen = std::collections::HashSet::new();
        let mut position: i64 = 0;
        for track in initial_tracks {
            if seen.insert(track.clone()) {
                conn.execute(
                    "INSERT INTO playlist_entries(playlist_id, position, track_id)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![id.0, position, track.0],
                )?;
                position += 1;
            }
        }
        Ok(id)
    }
}

impl SettingsStore for SqliteStore {
    /// Load every persisted setting from the typed tables. Missing values
    /// yield their defaults (see the port docs).
    fn load_settings(&self) -> Result<Settings, AppError> {
        let scalars = self
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT volume, advanced_mode, high_contrast, replaygain_enabled
                     FROM app_settings WHERE id = 1",
                    [],
                    |row| {
                        Ok(ScalarSettings {
                            volume: row.get(0)?,
                            advanced_mode: row.get::<_, i64>(1)? != 0,
                            high_contrast: row.get::<_, i64>(2)? != 0,
                            replaygain_enabled: row.get::<_, i64>(3)? != 0,
                        })
                    },
                )
            })
            .map_err(|e| AppError::InvalidOperation(format!("failed to load settings: {e}")))?;

        let library_paths = self.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM library_paths")?;
            let rows = stmt.query_map([], |row| Ok(PathBuf::from(row.get::<_, String>(0)?)))?;
            rows.collect()
        })?;

        let watch_states = self.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path, state, warning_message FROM watch_states")?;
            let mapped = stmt.query_map([], |row| {
                let path: String = row.get(0)?;
                let state_text: String = row.get(1)?;
                let warning_message: Option<String> = row.get(2)?;
                Ok((
                    PathBuf::from(path),
                    match state_text.as_str() {
                        "enabled" => WatchState::Enabled,
                        "warning" => WatchState::Warning(warning_message.unwrap_or_else(|| {
                            String::from("watching is unavailable for this path")
                        })),
                        _ => WatchState::Disabled,
                    },
                ))
            })?;
            mapped.collect()
        })?;

        Ok(Settings {
            scalars,
            library_paths,
            watch_states,
        })
    }

    /// One small durable transaction for the scalar block.
    fn save_scalars(&mut self, scalars: &ScalarSettings) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let result = conn.execute(
                "UPDATE app_settings
                 SET volume = ?1, advanced_mode = ?2, high_contrast = ?3,
                     replaygain_enabled = ?4
                 WHERE id = 1",
                rusqlite::params![
                    scalars.volume,
                    i64::from(scalars.advanced_mode),
                    i64::from(scalars.high_contrast),
                    i64::from(scalars.replaygain_enabled),
                ],
            );
            match result {
                Ok(_) => conn.execute_batch("COMMIT;"),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to save settings: {e}")))
    }

    /// Replace the library-path list in one small durable transaction.
    fn save_library_paths(&mut self, paths: &[std::path::PathBuf]) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let clear = conn.execute("DELETE FROM library_paths", []);
            let insert_all = clear.and_then(|_| {
                for p in paths {
                    conn.execute(
                        "INSERT INTO library_paths(path) VALUES (?1)",
                        [p.to_string_lossy().as_ref()],
                    )?;
                }
                Ok(())
            });
            match insert_all {
                Ok(()) => conn.execute_batch("COMMIT;"),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to save library paths: {e}")))
    }

    /// Replace the whole watch-state map in one small durable transaction.
    fn save_watch_states(&mut self, states: &HashMap<PathBuf, WatchState>) -> Result<(), AppError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let clear = conn.execute("DELETE FROM watch_states", []);
            let insert_all = clear.and_then(|_| {
                for (path, state) in states {
                    let (state_text, warning_message) = match state {
                        WatchState::Disabled => ("disabled", None),
                        WatchState::Enabled => ("enabled", None),
                        WatchState::Warning(reason) => ("warning", Some(reason.clone())),
                    };
                    conn.execute(
                        "INSERT INTO watch_states(path, state, warning_message) VALUES (?1, ?2, ?3)",
                        rusqlite::params![
                            path.to_string_lossy(),
                            state_text,
                            warning_message
                        ],
                    )?;
                }
                Ok(())
            });
            match insert_all {
                Ok(()) => conn.execute_batch("COMMIT;"),
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(e)
                }
            }
        })
        .map_err(|e| AppError::InvalidOperation(format!("failed to save watch states: {e}")))
    }
}

/// `SettingsStore` view of the shared store handle. One `SQLite` connection
/// serves every store port, so every call locks the shared connection for
/// the duration of its transaction.
pub struct MutexSettingsStore {
    store: std::sync::Arc<std::sync::Mutex<SqliteStore>>,
}

impl MutexSettingsStore {
    /// Wrap the shared store handle.
    #[must_use]
    pub fn new(store: std::sync::Arc<std::sync::Mutex<SqliteStore>>) -> Self {
        Self { store }
    }
}

impl SettingsStore for MutexSettingsStore {
    /// Load every persisted setting through the shared connection.
    fn load_settings(&self) -> Result<Settings, AppError> {
        self.store.lock_or_recover().load_settings()
    }

    /// Save the scalar block through the shared connection.
    fn save_scalars(&mut self, scalars: &ScalarSettings) -> Result<(), AppError> {
        self.store.lock_or_recover().save_scalars(scalars)
    }

    /// Replace the library-path list through the shared connection.
    fn save_library_paths(&mut self, paths: &[std::path::PathBuf]) -> Result<(), AppError> {
        self.store.lock_or_recover().save_library_paths(paths)
    }

    /// Replace the whole watch-state map through the shared connection.
    fn save_watch_states(&mut self, states: &HashMap<PathBuf, WatchState>) -> Result<(), AppError> {
        self.store.lock_or_recover().save_watch_states(states)
    }
}

/// `PlaylistStore` view of the shared store handle. One `SQLite` connection
/// serves every store port, so every call locks the shared connection for
/// the duration of its transaction.
pub struct MutexPlaylistStore {
    store: std::sync::Arc<std::sync::Mutex<SqliteStore>>,
}

impl MutexPlaylistStore {
    /// Wrap the shared store handle.
    #[must_use]
    pub fn new(store: std::sync::Arc<std::sync::Mutex<SqliteStore>>) -> Self {
        Self { store }
    }
}

impl PlaylistStore for MutexPlaylistStore {
    /// Load every Playlist through the shared connection.
    fn load_playlists(&self) -> Result<Vec<Playlist>, AppError> {
        self.store.lock_or_recover().load_playlists()
    }

    /// Create a Playlist through the shared connection.
    fn create_playlist(
        &mut self,
        name: &str,
        initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, AppError> {
        self.store
            .lock_or_recover()
            .create_playlist(name, initial_tracks)
    }

    /// Rename a Playlist through the shared connection.
    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, AppError> {
        self.store.lock_or_recover().rename_playlist(id, new_name)
    }

    /// Delete a Playlist through the shared connection.
    fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, AppError> {
        self.store.lock_or_recover().delete_playlist(id)
    }

    /// Append an entry through the shared connection.
    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Result<bool, AppError> {
        self.store.lock_or_recover().add_playlist_entry(id, track)
    }

    /// Remove entries through the shared connection.
    fn remove_playlist_entries(
        &mut self,
        id: &PlaylistId,
        track: &TrackId,
    ) -> Result<bool, AppError> {
        self.store
            .lock_or_recover()
            .remove_playlist_entries(id, track)
    }

    /// Reorder entries through the shared connection.
    fn reorder_playlist_entries(
        &mut self,
        id: &PlaylistId,
        ordered: &[TrackId],
    ) -> Result<bool, AppError> {
        self.store
            .lock_or_recover()
            .reorder_playlist_entries(id, ordered)
    }
}

/// `LibraryMutationStore` view of the shared store handle. One `SQLite`
/// connection serves every store port, so every call locks the shared
/// connection for the duration of its transaction.
#[derive(Clone)]
pub struct MutexLibraryMutationStore {
    store: std::sync::Arc<std::sync::Mutex<SqliteStore>>,
}

impl MutexLibraryMutationStore {
    /// Wrap the shared store handle.
    #[must_use]
    pub fn new(store: std::sync::Arc<std::sync::Mutex<SqliteStore>>) -> Self {
        Self { store }
    }
}

impl LibraryMutationStore for MutexLibraryMutationStore {
    /// Apply one scan batch through the shared connection.
    fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, AppError> {
        self.store.lock_or_recover().apply_scan_batch(tracks)
    }

    /// Record a finished play through the shared connection.
    fn record_track_played(
        &mut self,
        id: &TrackId,
        played_at: SystemTime,
    ) -> Result<bool, AppError> {
        self.store
            .lock_or_recover()
            .record_track_played(id, played_at)
    }

    /// Apply a tag refresh through the shared connection.
    fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), AppError> {
        self.store.lock_or_recover().apply_tag_refresh(track)
    }

    /// Remove a library root through the shared connection.
    fn remove_library_path(&mut self, root: &std::path::Path) -> Result<usize, AppError> {
        self.store.lock_or_recover().remove_library_path(root)
    }

    /// Wipe the Library collection through the shared connection.
    fn clear_library(&mut self) -> Result<usize, AppError> {
        self.store.lock_or_recover().clear_library()
    }
}

/// `LibraryQueryStore` view of the shared store handle. One `SQLite`
/// connection serves every store port, so every call locks the shared
/// connection for the duration of its query.
#[derive(Clone)]
pub struct MutexLibraryQueryStore {
    store: std::sync::Arc<std::sync::Mutex<SqliteStore>>,
}

impl MutexLibraryQueryStore {
    /// Wrap the shared store handle.
    #[must_use]
    pub fn new(store: std::sync::Arc<std::sync::Mutex<SqliteStore>>) -> Self {
        Self { store }
    }
}

impl LibraryQueryStore for MutexLibraryQueryStore {
    /// Resolve one Track through the shared connection.
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, AppError> {
        self.store.lock_or_recover().get_track(id)
    }

    /// Fetch one flat-list window through the shared connection.
    fn tracks_window(&self, offset: usize, limit: usize) -> Result<Vec<Track>, AppError> {
        self.store.lock_or_recover().tracks_window(offset, limit)
    }

    /// Count tracks through the shared connection.
    fn track_count(&self) -> Result<usize, AppError> {
        self.store.lock_or_recover().track_count()
    }

    /// Fetch one search window through the shared connection.
    fn search_window(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Track>, AppError> {
        self.store
            .lock_or_recover()
            .search_window(query, offset, limit)
    }

    /// Count matches through the shared connection.
    fn search_count(&self, query: &str) -> Result<usize, AppError> {
        self.store.lock_or_recover().search_count(query)
    }

    /// Load the full collection snapshot through the shared connection.
    fn load_collection(&self) -> Result<LibraryCollection, AppError> {
        self.store.lock_or_recover().load_collection()
    }

    /// List every artist through the shared connection.
    fn all_artists(&self) -> Result<Vec<Artist>, AppError> {
        self.store.lock_or_recover().all_artists()
    }

    /// List one artist's albums through the shared connection.
    fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, AppError> {
        self.store.lock_or_recover().artist_albums(artist)
    }

    /// List one album's tracks through the shared connection.
    fn album_tracks(&self, album_artist: &str, album_title: &str) -> Result<Vec<Track>, AppError> {
        self.store
            .lock_or_recover()
            .album_tracks(album_artist, album_title)
    }

    /// Probe a folder subtree through the shared connection.
    fn folder_has_audio(&self, folder: &std::path::Path) -> Result<bool, AppError> {
        self.store.lock_or_recover().folder_has_audio(folder)
    }

    /// Search one folder subtree through the shared connection.
    fn folder_has_search_match(
        &self,
        folder: &std::path::Path,
        query: &str,
    ) -> Result<bool, AppError> {
        self.store
            .lock_or_recover()
            .folder_has_search_match(folder, query)
    }

    /// List a folder subtree's track ids through the shared connection.
    fn track_ids_in_folder_tree(&self, folder: &std::path::Path) -> Result<Vec<TrackId>, AppError> {
        self.store
            .lock_or_recover()
            .track_ids_in_folder_tree(folder)
    }

    /// List a folder's direct tracks through the shared connection.
    fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, AppError> {
        self.store.lock_or_recover().tracks_in_folder(folder)
    }

    /// List a folder's child directories through the shared connection.
    fn subdirs_with_audio(&self, folder: &std::path::Path) -> Result<Vec<PathBuf>, AppError> {
        self.store.lock_or_recover().subdirs_with_audio(folder)
    }

    /// Compute one smart playlist through the shared connection.
    fn smart_playlist(
        &self,
        kind: SmartPlaylistKind,
        limit: usize,
    ) -> Result<Vec<Track>, AppError> {
        self.store.lock_or_recover().smart_playlist(kind, limit)
    }
}
