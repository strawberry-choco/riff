//! The `SQLite` `Application Store`.
//!
//! riff's single authoritative persistent state lives in one embedded `SQLite`
//! database. This module owns opening the database file, configuring the
//! connection for durability, and running the embedded, checksummed migration
//! set. Open or migrate failures are fatal startup errors surfaced as clear
//! [`StoreError`]s rather than silent fallbacks.

use crate::MutexExt;
use crossbeam_channel::Sender;
use riff_persistence::errors::StoreError;
use riff_persistence::playlist::{Playlist, PlaylistId};
use riff_persistence::store::{
    FullScanSummary, LOST_GEMS_THRESHOLD, LibraryCounts, LibraryMutationStore, LibraryQueryStore,
    PlaylistEntry, PlaylistStore, ScalarSettings, Settings, SettingsStore, SortDirection,
    StoreChanged, StoreGeneration, StoreMigrations, WatchState,
};
use riff_persistence::track::{
    Album, Artist, GenreCount, SmartPlaylistKind, Track, TrackId, TrackMetadata,
};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
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
    (
        "005_playback_prefs",
        "64cb710aa547f4fd5bdf57aacbc65f5444a7f256fc71b018f45eed80f0ca3a7c",
    ),
    (
        "006_track_favorites",
        "e19647f102d120af1640cb5bcd5475762eff791a244f94b40736a37955574652",
    ),
    (
        "007_browser_layout",
        "b862ca731be1c0c6f033537c462688cf9838dbc0c456f8992217769fd7e2c431",
    ),
    (
        "008_library_scan_prefs",
        "4f0d61c2b1a3e2f8c9d5e7a6b4c3d2e1f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5",
    ),
    (
        "009_smart_lists_collapsed",
        "c922ef2496e210e5a7c8aed8c43e0c027dfa55fe330801d6f5d88683984113ee",
    ),
    (
        "010_drop_missing_artwork_strategy",
        "276a52aa96ff1fa936a47b702bcfb923c971a9f6803fdd6b26ea270adbf7ca1a",
    ),
    (
        "011_entity_search_keys",
        "232d8e913877cee84852267eff8439eb997ce604532045002d97eb0346a275d0",
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
    Migration {
        version: 5,
        name: "005_playback_prefs",
        // Shuffle and repeat round-trip through the scalar settings row so
        // the player-bar toggles survive restarts. repeat_mode encodes the
        // cycle 0 = off, 1 = all, 2 = one (see `ScalarSettings`).
        sql: "ALTER TABLE app_settings
          ADD COLUMN shuffle INTEGER NOT NULL DEFAULT 0 CHECK (shuffle IN (0, 1));
        ALTER TABLE app_settings
          ADD COLUMN repeat_mode INTEGER NOT NULL DEFAULT 0 CHECK (repeat_mode IN (0, 1, 2));",
    },
    Migration {
        version: 6,
        name: "006_track_favorites",
        // The per-track favorite flag (design-handoff issue 03): a user
        // fact stored directly on the track row, so removing or clearing
        // the Library deletes the flag with the row — favorites can never
        // dangle. The scan/tag upserts list columns explicitly and leave
        // this one untouched, so rescans preserve it like play history.
        sql: "ALTER TABLE tracks
          ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1));",
    },
    Migration {
        version: 7,
        name: "007_browser_layout",
        // The library browser column's render mode (design-handoff issue
        // 06): the top bar's list/grid toggle persists here so the choice
        // survives restarts. 0 = list, 1 = grid.
        sql: "ALTER TABLE app_settings
          ADD COLUMN browser_layout INTEGER NOT NULL DEFAULT 0 CHECK (browser_layout IN (0, 1));",
    },
    Migration {
        version: 8,
        name: "008_library_scan_prefs",
        // The Library Scan preferences (design-handoff issue 12): hidden-file
        // skipping, the enabled audio formats (a comma-joined extension list
        // — extensions never contain commas — seeded to every format the
        // scanner has always indexed), embedded-artwork reading, and the
        // missing-artwork strategy. Defaults mirror the scanner's historical
        // behavior so existing stores keep scanning exactly as before.
        sql: "ALTER TABLE app_settings
          ADD COLUMN skip_hidden_files INTEGER NOT NULL DEFAULT 1 CHECK (skip_hidden_files IN (0, 1));
        ALTER TABLE app_settings
          ADD COLUMN scan_formats TEXT NOT NULL DEFAULT 'mp3,m4a,aac,opus,ogg,flac,wav';
        ALTER TABLE app_settings
          ADD COLUMN read_embedded_artwork INTEGER NOT NULL DEFAULT 1 CHECK (read_embedded_artwork IN (0, 1));
        ALTER TABLE app_settings
          ADD COLUMN missing_artwork_strategy TEXT NOT NULL DEFAULT 'generated_colour'
            CHECK (missing_artwork_strategy IN ('generated_colour'));",
    },
    Migration {
        version: 9,
        name: "009_smart_lists_collapsed",
        // The Smart Lists sidebar section's collapse toggle (persistent UI
        // preference): folded away means the section header shows only.
        // Mirrors the other scalar boolean display prefs; default 0 keeps
        // existing stores expanded.
        sql: "ALTER TABLE app_settings
          ADD COLUMN smart_lists_collapsed INTEGER NOT NULL DEFAULT 0 CHECK (smart_lists_collapsed IN (0, 1));",
    },
    Migration {
        version: 10,
        name: "010_drop_missing_artwork_strategy",
        // The missing-artwork strategy setting is removed (the generated
        // colour placeholder was dropped with the feature). SQLite refuses to
        // DROP a column used by a CHECK constraint, so the settings row is
        // rebuilt without the column; every other scalar carries over as-is.
        sql: "CREATE TABLE app_settings_new (
          id INTEGER PRIMARY KEY CHECK (id = 1),
          volume REAL,
          advanced_mode INTEGER NOT NULL DEFAULT 0 CHECK (advanced_mode IN (0, 1)),
          high_contrast INTEGER NOT NULL DEFAULT 0 CHECK (high_contrast IN (0, 1)),
          replaygain_enabled INTEGER NOT NULL DEFAULT 0 CHECK (replaygain_enabled IN (0, 1)),
          shuffle INTEGER NOT NULL DEFAULT 0 CHECK (shuffle IN (0, 1)),
          repeat_mode INTEGER NOT NULL DEFAULT 0 CHECK (repeat_mode IN (0, 1, 2)),
          browser_layout INTEGER NOT NULL DEFAULT 0 CHECK (browser_layout IN (0, 1)),
          skip_hidden_files INTEGER NOT NULL DEFAULT 1 CHECK (skip_hidden_files IN (0, 1)),
          scan_formats TEXT NOT NULL DEFAULT 'mp3,m4a,aac,opus,ogg,flac,wav',
          read_embedded_artwork INTEGER NOT NULL DEFAULT 1 CHECK (read_embedded_artwork IN (0, 1)),
          smart_lists_collapsed INTEGER NOT NULL DEFAULT 0 CHECK (smart_lists_collapsed IN (0, 1))
        );
        INSERT INTO app_settings_new (
          id, volume, advanced_mode, high_contrast, replaygain_enabled,
          shuffle, repeat_mode, browser_layout, skip_hidden_files, scan_formats,
          read_embedded_artwork, smart_lists_collapsed
        )
        SELECT
          id, volume, advanced_mode, high_contrast, replaygain_enabled,
          shuffle, repeat_mode, browser_layout, skip_hidden_files, scan_formats,
          read_embedded_artwork, smart_lists_collapsed
        FROM app_settings;
        DROP TABLE app_settings;
        ALTER TABLE app_settings_new RENAME TO app_settings;",
    },
    Migration {
        version: 11,
        name: "011_entity_search_keys",
        // Write-time-lowercased key columns on the entity tables
        // (`artists.name_lower`, `albums.album_artist_lower` /
        // `albums.title_lower`) so entity *name* matching can be
        // case-insensitive: SQLite's `instr` is case-sensitive, and the
        // tracks' derived `search_text` is the same write-time-lowercase
        // precedent. Scans/tag-edits write the Rust-lowercased value (which
        // folds non-Latin correctly); the SQL backfill here covers rows that
        // predate the migration (ASCII folding only — a rescan refreshes
        // non-Latin rows, the same contract as `search_text`).
        sql: "ALTER TABLE artists ADD COLUMN name_lower TEXT NOT NULL DEFAULT ''; \
              ALTER TABLE albums ADD COLUMN album_artist_lower TEXT NOT NULL DEFAULT ''; \
              ALTER TABLE albums ADD COLUMN title_lower TEXT NOT NULL DEFAULT ''; \
              UPDATE artists SET name_lower = lower(name); \
              UPDATE albums SET album_artist_lower = lower(album_artist); \
              UPDATE albums SET title_lower = lower(title);",
    },
];

/// Location of the Application Store database file: `riff.sqlite3` in the
/// data-local directory, mirroring where the legacy JSON files lived.
///
/// # Errors
/// Returns an error when the platform provides no data-local directory;
/// callers treat this as a fatal startup condition.
pub fn default_store_path() -> Result<std::path::PathBuf, StoreError> {
    directories::ProjectDirs::from("", "", "riff")
        .map(|dirs| dirs.data_local_dir().join("riff.sqlite3"))
        .ok_or_else(|| {
            StoreError::InvalidOperation(
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
/// Missing files are not an error; failures surface as [`StoreError`].
fn rename_aside(path: &std::path::Path) -> Result<(), StoreError> {
    if !path.exists() {
        return Ok(());
    }
    let mut suffixed = path.as_os_str().to_owned();
    suffixed.push(format!(".{}", unix_nanoseconds()));
    std::fs::rename(path, &suffixed).map_err(|e| {
        StoreError::InvalidOperation(format!(
            "failed to set aside corrupted store file {}: {e}",
            path.display()
        ))
    })
}

/// The shared `SQLite` connection backing the `Application Store`.
///
/// A cheaply clonable handle: every clone shares the one mutex-guarded
/// connection and both session generations, so any clone can serve any of
/// the store ports.
#[derive(Clone)]
pub struct SqliteStore {
    conn: std::sync::Arc<std::sync::Mutex<Connection>>,
    library_generation: StoreGeneration,
    playlist_generation: StoreGeneration,
    /// Sender for `BackendEvents`-side change notifications. Writes are
    /// best-effort (`send()` — dropped receivers are fine) and happen
    /// exactly beside the corresponding generation bump.
    changes: Sender<StoreChanged>,
}

impl SqliteStore {
    /// Open (creating it when missing) the store at `path`, configure the
    /// connection for durability, and apply every pending migration.
    ///
    /// `changes_tx` is the sender over which this store emits
    /// [`StoreChanged`] notifications beside each generation bump
    /// (emit-beside-bump, issue 04). Clones share the sender; a dropped
    /// receiver simply suppresses future notifications.
    pub fn open_and_migrate(
        path: &std::path::Path,
        changes_tx: Sender<StoreChanged>,
    ) -> Result<Self, StoreError> {
        match Self::try_open_and_migrate(path, changes_tx.clone()) {
            Ok(store) => Ok(store),
            Err(failure) => Self::recover_from_failure(path, failure, changes_tx),
        }
    }

    /// Open the store for writing, configure it, and migrate it. Called after
    /// the read-only integrity probe passed (or the file was absent).
    fn open_writable_and_migrate(
        path: &std::path::Path,
        changes_tx: Sender<StoreChanged>,
    ) -> Result<Self, StoreError> {
        let conn = Connection::open(path).map_err(|e| {
            StoreError::InvalidOperation(format!(
                "failed to open Application Store at {}: {e}",
                path.display()
            ))
        })?;
        Self::configure_and_migrate(conn, changes_tx)
    }

    /// Best-effort open without recovery; the error carries the exact stage
    /// (open, integrity check, or migration) that failed.
    fn try_open_and_migrate(
        path: &std::path::Path,
        changes_tx: Sender<StoreChanged>,
    ) -> Result<Self, StoreError> {
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
                        return Self::open_writable_and_migrate(path, changes_tx);
                    }
                    return Err(StoreError::InvalidOperation(format!(
                        "Application Store at {} failed integrity check: {result}",
                        path.display()
                    )));
                }
                // quick_check itself errored (e.g. unreadable schema):
                // treat as corrupt and fail through the recovery path.
                Err(StoreError::InvalidOperation(format!(
                    "Application Store at {} failed integrity check (quick_check error)",
                    path.display()
                )))
            }
            Err(open_err) if path.exists() => Err(StoreError::InvalidOperation(format!(
                "Application Store at {} failed integrity check: {open_err}",
                path.display()
            ))),
            Err(_missing_file) => {
                // A missing file is a normal fresh start; any other state is
                // handled by the arms above.
                Self::open_writable_and_migrate(path, changes_tx)
            }
        }
    }

    /// Automatic corruption recovery: when opening, checking, or migrating
    /// the store fails, the database file and its `-wal`/`-shm` siblings are
    /// renamed aside (Unix-nanosecond suffixed, preserved for recovery tools)
    /// and a fresh store is created. Only a failure of the recovery itself is
    /// a fatal startup error.
    fn recover_from_failure(
        path: &std::path::Path,
        failure: StoreError,
        changes_tx: Sender<StoreChanged>,
    ) -> Result<Self, StoreError> {
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
                    return Err(StoreError::InvalidOperation(format!(
                        "fatal: failed to inspect corrupted store file {}: {e}",
                        sibling.to_string_lossy()
                    )));
                }
            };
            if existed {
                rename_aside(std::path::Path::new(&sibling))?;
            }
        }
        match Self::try_open_and_migrate(path, changes_tx) {
            Ok(store) => Ok(store),
            Err(recovery_failure) => Err(StoreError::InvalidOperation(format!(
                "fatal: Application Store at {} could not be recovered after {}: {recovery_failure}",
                path.display(),
                failure
            ))),
        }
    }

    /// Apply every pending migration to the already-open store. Idempotent:
    /// applied versions are verified against their embedded checksum and
    /// skipped, pending ones are applied exactly once.
    pub fn apply_migrations(&mut self) -> Result<(), StoreError> {
        let conn = self.conn.lock_or_recover();
        Self::run_migrations(&conn)
    }

    /// Run `f` with the underlying connection, locking it for the duration
    /// of the call (recovering from a poisoned lock rather than panicking).
    /// Used by infrastructure tests at the port boundary; later store
    /// features build on this access path.
    pub fn with_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        let conn = self.conn.lock_or_recover();
        f(&conn).map_err(|e| StoreError::InvalidOperation(format!("store query failed: {e}")))
    }

    /// The session Library generation this handle bumps after each committed
    /// Library mutation (ADR 0002). Clones share it.
    #[must_use]
    pub fn library_generation(&self) -> StoreGeneration {
        self.library_generation.clone()
    }

    /// The session playlist generation this handle bumps after each
    /// committed playlist mutation (ADR 0002), independent of the Library
    /// generation so entry edits never invalidate Library projections.
    /// Clones share it.
    #[must_use]
    pub fn playlist_generation(&self) -> StoreGeneration {
        self.playlist_generation.clone()
    }

    /// Record whether a playlist mutation committed by bumping the session
    /// playlist generation exactly on commit (ADR 0002), then push a
    /// [`StoreChanged::Playlists`] beside the bump. The emit happens inside
    /// this method so callers cannot forget it; best-effort (`send()`),
    /// dropped receivers are fine.
    fn bump_playlist_generation_on_commit(&self, committed: bool) {
        if committed {
            let generation = self.playlist_generation.bump();
            let _ = self.changes.send(StoreChanged::Playlists(generation));
        }
    }

    /// Record whether a Library mutation committed by bumping the session
    /// Library generation exactly on commit (ADR 0002), then push a
    /// [`StoreChanged::Library`] beside the bump. The emit happens inside
    /// this method so callers cannot forget it; best-effort (`send()`),
    /// dropped receivers are fine.
    fn bump_library_generation_on_commit(&self, committed: bool) {
        if committed {
            let generation = self.library_generation.bump();
            let _ = self.changes.send(StoreChanged::Library(generation));
        }
    }

    fn configure_and_migrate(
        mut conn: Connection,
        changes: Sender<StoreChanged>,
    ) -> Result<Self, StoreError> {
        Self::configure_connection(&mut conn)?;
        Self::run_migrations(&conn)?;
        Ok(Self {
            conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
            library_generation: StoreGeneration::new(),
            playlist_generation: StoreGeneration::new(),
            changes,
        })
    }

    /// Handle for wiring this store into the event backbone. Returns a fresh
    /// crossbeam [`Sender`] of [`StoreChanged`] so tests and the event inbox can
    /// consume the notifications this handle produces beside each generation bump.
    #[must_use]
    pub fn changes_sender(&self) -> Sender<StoreChanged> {
        self.changes.clone()
    }

    /// Durability setup required by the spec: WAL journal mode,
    /// synchronous=NORMAL, foreign keys ON, and a short busy timeout.
    fn configure_connection(conn: &mut Connection) -> Result<(), StoreError> {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| StoreError::InvalidOperation(format!("failed to enable WAL mode: {e}")))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to set synchronous=NORMAL: {e}"))
            })?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to enable foreign keys: {e}"))
            })?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to set busy timeout: {e}"))
            })?;
        // Folder prefix queries match paths byte-for-byte like the former
        // in-memory `Path::starts_with` checks; SQLite's default ASCII case
        // folding for LIKE would silently widen every folder query.
        conn.pragma_update(None, "case_sensitive_like", "ON")
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to enable case-sensitive LIKE: {e}"))
            })?;
        Ok(())
    }

    /// Bring the schema up to date by applying every pending migration in
    /// order. Each migration commits atomically with its bookkeeping row;
    /// any failure rolls back that migration completely (nothing partially
    /// applies) and aborts startup.
    fn run_migrations(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                checksum TEXT NOT NULL,
                name TEXT NOT NULL UNIQUE,
                applied_at INTEGER NOT NULL
             );",
        )
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to prepare schema_migrations table: {e}"))
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
                    StoreError::InvalidOperation(format!(
                        "failed to read migration state for version {}: {e}",
                        migration.version
                    ))
                })?;
            if let Some(recorded) = already_applied {
                if recorded != expected_checksum {
                    return Err(StoreError::InvalidOperation(format!(
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
                StoreError::InvalidOperation(format!(
                    "failed to apply migration {} ({}): {e}",
                    migration.version, migration.name
                ))
            })?;
        }
        Ok(())
    }
}

impl StoreMigrations for SqliteStore {
    fn open_and_migrate(&self, path: &std::path::Path) -> Result<(), StoreError> {
        Self::open_and_migrate(path, self.changes.clone()).map(|_| ())
    }
}

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
    fn load_playlists(&self) -> Result<Vec<Playlist>, StoreError> {
        self.with_connection(|conn| {
            let mut stmt =
                conn.prepare_cached("SELECT id, name, created_at FROM playlists ORDER BY rowid")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            // Prepared once for the whole loop (allocation plan 4.1): the
            // per-playlist entries query no longer re-prepares per row.
            let mut entries = conn.prepare_cached(
                "SELECT track_id FROM playlist_entries
                 WHERE playlist_id = ?1 ORDER BY position",
            )?;
            let mut playlists = Vec::new();
            for row in rows {
                let (id, name, created_at) = row?;
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to load playlists: {e}")))
    }

    /// One Playlist's entries in playlist order, each with its Library
    /// validity from a LEFT JOIN against tracks: `valid` is true exactly
    /// when the referenced track row exists. Dangling references stay
    /// listed with their track unset (ADR 0001); unknown playlist ids yield
    /// an empty `Vec`.
    fn load_playlist_entries(&self, id: &PlaylistId) -> Result<Vec<PlaylistEntry>, StoreError> {
        self.with_connection(|conn| {
            // The two tables share no column names, so the unqualified
            // [`TRACK_COLUMNS`] resolve to `tracks`; the trailing
            // `e.track_id` (index [`TRACK_COLUMN_COUNT`]) is the entry's own
            // reference, which survives the join even when dangling. A
            // dangling row has NULL in every tracks column, so `path`
            // (index 0) being NULL is the validity bit.
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {TRACK_COLUMNS}, e.track_id
                 FROM playlist_entries e
                 LEFT JOIN tracks ON tracks.path = e.track_id
                 WHERE e.playlist_id = ?1
                 ORDER BY e.position"
            ))?;
            let rows = stmt.query_map([&id.0], |row| {
                let entry_id: String = row.get(TRACK_COLUMN_COUNT)?;
                let track = if row.get::<_, Option<String>>(0)?.is_some() {
                    Some(track_from_row(row)?)
                } else {
                    None
                };
                Ok(PlaylistEntry {
                    valid: track.is_some(),
                    id: TrackId(entry_id),
                    track,
                })
            })?;
            rows.collect()
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to load playlist entries: {e}")))
    }

    /// One immediate durable transaction: the playlist row plus its initial
    /// entries commit together or not at all.
    fn create_playlist(
        &mut self,
        name: &str,
        initial_tracks: &[TrackId],
    ) -> Result<PlaylistId, StoreError> {
        let created = self
            .with_connection(|conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                match Self::create_playlist_in_tx(conn, name, initial_tracks) {
                    Ok(id) => conn.execute_batch("COMMIT;").map(|()| id),
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK;");
                        Err(e)
                    }
                }
            })
            .map_err(|e| StoreError::InvalidOperation(format!("failed to create playlist: {e}")));
        self.bump_playlist_generation_on_commit(created.is_ok());
        created
    }

    /// One immediate durable transaction: only the renamed row is written.
    fn rename_playlist(&mut self, id: &PlaylistId, new_name: &str) -> Result<bool, StoreError> {
        let renamed = self
            .with_connection(|conn| {
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
            .map_err(|e| StoreError::InvalidOperation(format!("failed to rename playlist: {e}")));
        self.bump_playlist_generation_on_commit(matches!(renamed, Ok(true)));
        renamed
    }

    /// One immediate durable transaction: the playlist row goes and its
    /// entries cascade; no other playlist's data is rewritten.
    fn delete_playlist(&mut self, id: &PlaylistId) -> Result<bool, StoreError> {
        let deleted = self
            .with_connection(|conn| {
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
            .map_err(|e| StoreError::InvalidOperation(format!("failed to delete playlist: {e}")));
        self.bump_playlist_generation_on_commit(matches!(deleted, Ok(true)));
        deleted
    }

    /// One immediate durable transaction: a single appended entry row, or
    /// nothing when the playlist is unknown or the entry already exists.
    fn add_playlist_entry(&mut self, id: &PlaylistId, track: &TrackId) -> Result<bool, StoreError> {
        let added = self
            .with_connection(|conn| {
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
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to add playlist entry: {e}"))
            });
        self.bump_playlist_generation_on_commit(matches!(added, Ok(true)));
        added
    }

    /// One immediate durable transaction removing every occurrence of the
    /// Track reference from the playlist's entries.
    fn remove_playlist_entries(
        &mut self,
        id: &PlaylistId,
        track: &TrackId,
    ) -> Result<bool, StoreError> {
        let removed = self
            .with_connection(|conn| {
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
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to remove playlist entries: {e}"))
            });
        self.bump_playlist_generation_on_commit(matches!(removed, Ok(true)));
        removed
    }

    /// One immediate durable transaction rewriting the playlist's entries to
    /// exactly `ordered` (positions 0..n): the delete and every reinsert
    /// commit together or not at all, so a crash mid-reorder can never leave
    /// a truncated or duplicated entry list.
    fn reorder_playlist_entries(
        &mut self,
        id: &PlaylistId,
        ordered: &[TrackId],
    ) -> Result<bool, StoreError> {
        let reordered = self
            .with_connection(|conn| {
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
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to reorder playlist entries: {e}"))
            });
        self.bump_playlist_generation_on_commit(matches!(reordered, Ok(true)));
        reordered
    }
}

impl LibraryMutationStore for SqliteStore {
    /// One immediate durable transaction per batch: parents are created
    /// first (album identity `(album artist, title)`, year/genre from the
    /// first-added track), then every track upserts by path with its play
    /// history columns excluded from the update branch, so rescans refresh
    /// metadata without ever touching history.
    fn apply_scan_batch(&mut self, tracks: &[Track]) -> Result<usize, StoreError> {
        let written = self
            .with_connection(|conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                match Self::apply_scan_batch_in_tx(conn, tracks) {
                    Ok(written) => conn.execute_batch("COMMIT;").map(|()| written),
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK;");
                        Err(e)
                    }
                }
            })
            .map_err(|e| StoreError::InvalidOperation(format!("failed to apply scan batch: {e}")));
        self.bump_library_generation_on_commit(written.is_ok());
        written
    }

    /// One immediate durable transaction per finished play: a single-row
    /// update bumps `play_count` and stamps `last_played` together, so a
    /// crash right afterward cannot lose the play.
    fn record_track_played(
        &mut self,
        id: &TrackId,
        played_at: SystemTime,
    ) -> Result<bool, StoreError> {
        let recorded = self
            .with_connection(|conn| {
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
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to record played track: {e}"))
            });
        self.bump_library_generation_on_commit(matches!(recorded, Ok(true)));
        recorded
    }

    /// One immediate durable transaction per favorite toggle: the
    /// single-row update commits alone, so a crash right after a toggle
    /// cannot lose it. The `favorite != ?1` guard makes a redundant set a
    /// no-op — nothing is written and no projection is invalidated.
    fn set_track_favorite(&mut self, id: &TrackId, favorite: bool) -> Result<bool, StoreError> {
        let set = self
            .with_connection(|conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                let updated = conn.execute(
                    "UPDATE tracks SET favorite = ?1
                     WHERE path = ?2 AND favorite != ?1",
                    rusqlite::params![favorite, id.0],
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
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to set the favorite flag: {e}"))
            });
        self.bump_library_generation_on_commit(matches!(set, Ok(true)));
        set
    }

    /// One immediate durable transaction for a tag edit: the metadata upsert
    /// (history columns excluded) plus the album year/genre re-derivation and
    /// orphan cleanup commit together or not at all.
    fn apply_tag_refresh(&mut self, track: &Track) -> Result<(), StoreError> {
        let applied = self
            .with_connection(|conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                match Self::apply_tag_refresh_in_tx(conn, track) {
                    Ok(()) => conn.execute_batch("COMMIT;"),
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK;");
                        Err(e)
                    }
                }
            })
            .map_err(|e| StoreError::InvalidOperation(format!("failed to apply tag refresh: {e}")));
        self.bump_library_generation_on_commit(applied.is_ok());
        applied
    }

    /// One immediate durable transaction removing exactly the root's tracks,
    /// their orphaned parents, and the root's own library-path record.
    /// Playlist entries are deliberately untouched — dangling references are
    /// valid product behavior.
    fn remove_library_path(&mut self, root: &std::path::Path) -> Result<usize, StoreError> {
        let root_text = root.to_string_lossy().into_owned();
        let removed = self
            .with_connection(|conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                // Byte-prefix match mirroring `Path::starts_with`: the root
                // itself (exact match) or the root followed by a path
                // separator, so "m:\music" can never swallow "m:\music2\...".
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
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to remove library path: {e}"))
            });
        self.bump_library_generation_on_commit(removed.is_ok());
        removed
    }

    /// One immediate durable transaction: all tracks (history included),
    /// then every album and artist left without tracks via the shared
    /// orphan cleanup. Playlists and Settings tables are never touched. Any
    /// failure rolls the whole wipe back — nothing partially clears.
    fn clear_library(&mut self) -> Result<usize, StoreError> {
        let cleared = self
            .with_connection(|conn| {
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
            .map_err(|e| StoreError::InvalidOperation(format!("failed to clear the library: {e}")));
        self.bump_library_generation_on_commit(cleared.is_ok());
        cleared
    }

    /// Overwrite the last-scan summary in `store_metadata` as ONE immediate
    /// durable transaction, then bump + notify the session Library
    /// generation beside the commit. The value packs
    /// `{finished nanos}:{files}:{errors}` in one string so the summary is
    /// as atomic as the timestamp ever was (design-handoff issue 12).
    fn record_full_scan_completed(&mut self, summary: FullScanSummary) -> Result<(), StoreError> {
        let nanos = nanos_from_system_time(summary.at);
        let value = format!("{}:{}:{}", nanos, summary.files, summary.errors);
        let committed = self
            .with_connection(|conn| {
                conn.execute_batch("BEGIN IMMEDIATE;")?;
                let outcome = conn.execute(
                    "INSERT INTO store_metadata (key, value) VALUES ('last_full_scan', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [value],
                );
                match outcome {
                    Ok(_) => conn.execute_batch("COMMIT;"),
                    Err(e) => {
                        let _ = conn.execute_batch("ROLLBACK;");
                        Err(e)
                    }
                }
            })
            .map_err(|e| {
                StoreError::InvalidOperation(format!("failed to record the last scan: {e}"))
            });
        self.bump_library_generation_on_commit(committed.is_ok());
        committed
    }
}

/// UTC epoch-nanosecond integer encoding for a `Duration` (the store's
/// on-disk format for track durations), saturating at `i64::MAX`.
fn duration_to_nanos(duration: Duration) -> i64 {
    i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX)
}

/// The independent genre entries of a stored genre string: split on `;`,
/// trim surrounding whitespace, and drop empty segments. A tag like
/// `"Rock; Jazz"` contributes the two entries `Rock` and `Jazz`.
fn genre_segments(genre: &str) -> impl Iterator<Item = &str> {
    genre.split(';').map(str::trim).filter(|g| !g.is_empty())
}

/// Whether a stored (possibly multi-value) genre string carries `target`
/// as one of its independent entries. Segment boundaries make this exact:
/// `"Indie Rock"` never matches `"Rock"` even though it contains the
/// substring.
fn genre_contains(stored: &str, target: &str) -> bool {
    genre_segments(stored).any(|g| g == target)
}

impl SqliteStore {
    /// Scan-batch body shared with the transaction wrapper above.
    fn apply_scan_batch_in_tx(conn: &Connection, tracks: &[Track]) -> rusqlite::Result<usize> {
        let mut written = 0;
        for track in tracks {
            // Resolved display fallbacks drive grouping and the FK chain;
            // raw optional metadata is stored alongside for exact
            // round-trips (search parity uses the raw values). The display
            // keys are owned copies — the scan path is cold (10-track
            // batches), and owned `String`s bind straight into the queries.
            let album_artist_key = track.metadata.display_album_artist().into_owned();
            let album_title_key = track.metadata.display_album().into_owned();
            conn.execute(
                "INSERT OR IGNORE INTO artists(name, name_lower) VALUES (?1, ?2)",
                rusqlite::params![&album_artist_key, album_artist_key.to_lowercase()],
            )?;
            // OR IGNORE keeps the first-added track's year/genre derivation.
            conn.execute(
                "INSERT OR IGNORE INTO albums(
                    album_artist, title, year, genre,
                    album_artist_lower, title_lower
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    album_artist_key,
                    album_title_key,
                    track.metadata.year.map(i64::from),
                    track.metadata.genre,
                    album_artist_key.to_lowercase(),
                    album_title_key.to_lowercase(),
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
            track.metadata.display_album_artist().into_owned(),
            track.metadata.display_album().into_owned(),
        );
        let mut affected: Vec<(String, String)> = Vec::with_capacity(2);
        if let Some(old) = previous_keys
            && old != new_key
        {
            affected.push(old);
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
            play_count, last_played_nanos, date_added_nanos, search_text, favorite";

/// How many columns [`TRACK_COLUMNS`] expands to; result rows that append
/// extra columns after them (e.g. the playlist-entries LEFT JOIN) index
/// past this.
const TRACK_COLUMN_COUNT: usize = 21;

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
    // A bare root ("/", "\\", "//") already ends in its separator, so the
    // continuation must not append another one: `/` + `/tmp/x` is
    // `/tmp/x`, not `//tmp/x`. `Path::starts_with` agrees — every absolute
    // path starts_with(`/`).
    if !trimmed.is_empty() && trimmed.chars().all(|c| c == '\\' || c == '/') {
        [
            trimmed.to_string(),
            format!("{escaped}%"),
            format!("{escaped}%"),
        ]
    } else {
        [
            trimmed.to_string(),
            format!("{escaped}/%"),
            format!("{escaped}\\%"),
        ]
    }
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
        favorite: row.get(20)?,
        search_text: row.get(19)?,
    })
}

impl LibraryQueryStore for SqliteStore {
    /// Resolve one `Track` by its `TrackId` (its full file path).
    fn get_track(&self, id: &TrackId) -> Result<Option<Track>, StoreError> {
        self.with_connection(|conn| {
            // Runs per finished play — cached (allocation plan 4.1).
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks WHERE path = ?1"
            ))?;
            let mut rows = stmt.query_map([&id.0], track_from_row)?;
            rows.next().transpose()
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to resolve track: {e}")))
    }

    /// One bounded window of the flat library list, path-ascending.
    fn tracks_window(&self, offset: usize, limit: usize) -> Result<Vec<Track>, StoreError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(&format!(
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list tracks: {e}")))
    }

    fn track_count(&self) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM tracks", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count tracks: {e}")))
    }

    /// Every Track id, path-ascending — the canonical flat ordering
    /// (ADR 0003) Queue Fill loads into the Playback Queue.
    fn all_track_ids(&self) -> Result<Vec<TrackId>, StoreError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT path FROM tracks ORDER BY path ASC")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0).map(TrackId))?;
            rows.collect()
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list track ids: {e}")))
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
    ) -> Result<Vec<Track>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            // Runs per keystroke — cached (allocation plan 4.1).
            let mut stmt = conn.prepare_cached(&format!(
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to search tracks: {e}")))
    }

    fn search_count(&self, query: &str) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            conn.query_row(
                "SELECT COUNT(*) FROM tracks WHERE instr(search_text, ?1) > 0",
                [needle],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count matches: {e}")))
    }

    /// Every artist name-ascending (byte-wise, matching the former UI sort
    /// over the in-memory mirror), each carrying its album keys in canonical
    /// browsing order. Two ordered reads; grouping happens in Rust so the
    /// per-artist album order survives.
    fn all_artists(&self) -> Result<Vec<Artist>, StoreError> {
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list artists: {e}")))
    }

    /// One artist's albums newest-first (missing year last) then title,
    /// each with its track ids in album-track order so an expanded artist
    /// renders without per-album queries.
    fn artist_albums(&self, artist: &str) -> Result<Vec<Album>, StoreError> {
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list artist albums: {e}")))
    }

    /// One album's tracks in full: track number ascending with missing
    /// numbers first (the legacy `unwrap_or(0)` slot), path tiebreak.
    fn album_tracks(
        &self,
        album_artist: &str,
        album_title: &str,
    ) -> Result<Vec<Track>, StoreError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks
                 WHERE album_artist_key = ?1 AND album_title_key = ?2
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC"
            ))?;
            let rows =
                stmt.query_map(rusqlite::params![album_artist, album_title], track_from_row)?;
            rows.collect()
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list album tracks: {e}")))
    }

    /// The Library-count totals in one query: scalar subselects over
    /// tracks, artists, and albums, with the genre count matching
    /// [`Self::genre_counts`] semantics (distinct non-empty per-track
    /// genre entries — semicolon-separated tags count once per entry).
    fn library_counts(&self) -> Result<LibraryCounts, StoreError> {
        self.with_connection(|conn| {
            let (tracks, artists, albums): (i64, i64, i64) = conn.query_row(
                "SELECT
                    (SELECT COUNT(*) FROM tracks),
                    (SELECT COUNT(*) FROM artists),
                    (SELECT COUNT(*) FROM albums)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            // Genre totals cannot be a scalar subselect: the distinct count
            // is over the split entries, so the raw tags aggregate in Rust.
            let mut stmt =
                conn.prepare("SELECT genre FROM tracks WHERE genre IS NOT NULL AND genre != ''")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut genres = std::collections::HashSet::new();
            for row in rows {
                for segment in genre_segments(&row?) {
                    genres.insert(segment.to_string());
                }
            }
            Ok(LibraryCounts {
                tracks: usize::try_from(tracks).expect("a track count fits in usize"),
                artists: usize::try_from(artists).expect("an artist count fits in usize"),
                albums: usize::try_from(albums).expect("an album count fits in usize"),
                genres: genres.len(),
            })
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count the library: {e}")))
    }

    /// Every genre entry name-ascending with its per-track count, aggregated
    /// from the tracks' own genre metadata — semicolon-separated tags count
    /// once per entry (a track tagged `"Rock; Jazz"` contributes one count
    /// to `Rock` and one to `Jazz`). Missing (`NULL`) and empty genre tags
    /// aggregate into nothing; entries sort byte-wise, matching the former
    /// `SQLite` BINARY-collation grouping.
    fn genre_counts(&self) -> Result<Vec<GenreCount>, StoreError> {
        self.with_connection(|conn| {
            // `GROUP BY` over the raw column would keep `"Rock; Jazz"` as one
            // row; the entries split in Rust instead.
            let mut stmt = conn.prepare(
                "SELECT genre FROM tracks
                 WHERE genre IS NOT NULL AND genre != ''",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut counts: HashMap<String, usize> = HashMap::new();
            for row in rows {
                for segment in genre_segments(&row?) {
                    *counts.entry(segment.to_string()).or_insert(0) += 1;
                }
            }
            let mut entries: Vec<GenreCount> = counts
                .into_iter()
                .map(|(genre, tracks)| GenreCount { genre, tracks })
                .collect();
            entries.sort_by(|a, b| a.genre.cmp(&b.genre));
            Ok(entries)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to aggregate genres: {e}")))
    }

    /// Artists name-ascending having at least one track with `genre`, each
    /// with the album keys of albums holding at least one matching track.
    /// Two ordered reads over the genre-bearing track rows; the per-track
    /// membership split (`;`-separated entries) happens in Rust so the
    /// per-artist album order survives, mirroring [`Self::all_artists`]. The
    /// JOIN against albums supplies the canonical ordering columns;
    /// consecutive-dedup collapses multi-track albums.
    fn artists_in_genre(&self, genre: &str) -> Result<Vec<Artist>, StoreError> {
        self.with_connection(|conn| {
            // No `DISTINCT`: the genre match is per track, so the SQL keeps
            // every row and the filtered stream dedups below. SQL ordering
            // keeps one album's rows contiguous, so the dedup preserves the
            // canonical artist/album order.
            let mut stmt = conn.prepare(
                "SELECT t.album_artist_key, a.title, t.genre
                 FROM tracks t
                 JOIN albums a ON a.album_artist = t.album_artist_key
                              AND a.title = t.album_title_key
                 WHERE t.genre IS NOT NULL AND t.genre != ''
                 ORDER BY t.album_artist_key ASC, COALESCE(a.year, 0) DESC, a.title ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut keys_by_artist: HashMap<String, Vec<String>> = HashMap::new();
            let mut last_key: Option<(String, String)> = None;
            for row in rows {
                let (artist, title, stored_genre) = row?;
                if !genre_contains(&stored_genre, genre) {
                    continue;
                }
                let key = (artist.clone(), title);
                if last_key.as_ref() == Some(&key) {
                    continue;
                }
                last_key = Some(key.clone());
                keys_by_artist
                    .entry(artist)
                    .or_default()
                    .push(format!("{} - {}", key.0, key.1));
            }

            let mut stmt = conn.prepare(
                "SELECT DISTINCT album_artist_key FROM tracks
                 WHERE genre IS NOT NULL AND genre != ''
                 ORDER BY album_artist_key ASC",
            )?;
            let names = stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            let mut artists = Vec::new();
            for name in names {
                if let Some(albums) = keys_by_artist.remove(&name) {
                    artists.push(Artist { name, albums });
                }
            }
            Ok(artists)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list genre artists: {e}")))
    }

    /// One artist's albums holding at least one track with `genre`, in
    /// canonical browsing order, each carrying only its matching track ids
    /// in album-track order. Mirrors [`Self::artist_albums`] with the genre
    /// filter applied to the membership read — membership is the split
    /// (`;`-separated) per-track entries, matched in Rust; albums left with
    /// no matching track are dropped.
    fn artist_albums_in_genre(&self, artist: &str, genre: &str) -> Result<Vec<Album>, StoreError> {
        self.with_connection(|conn| {
            // No `EXISTS` subquery: the per-track genre membership is split
            // in Rust below, so this lists the artist's albums unconditionally
            // and the final filtering keeps only those with a matching track.
            let mut stmt = conn.prepare(
                "SELECT title, year, genre FROM albums
                 WHERE album_artist = ?1
                 ORDER BY COALESCE(year, 0) DESC, title ASC",
            )?;
            let mut albums: Vec<Album> = stmt
                .query_map(rusqlite::params![artist], |row| {
                    Ok(Album {
                        artist: artist.to_string(),
                        title: row.get(0)?,
                        year: narrow_u32(row.get(1)?),
                        genre: row.get(2)?,
                        tracks: Vec::new(),
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            // Matching membership arrives globally ordered by number-then-
            // path; appending per album preserves that order inside each
            // album.
            let mut stmt = conn.prepare(
                "SELECT album_title_key, path, genre FROM tracks
                 WHERE album_artist_key = ?1 AND genre IS NOT NULL AND genre != ''
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC",
            )?;
            let members = stmt.query_map(rusqlite::params![artist], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut membership: HashMap<String, Vec<TrackId>> = HashMap::new();
            for member in members {
                let (title, path, stored_genre) = member?;
                if genre_contains(&stored_genre, genre) {
                    membership.entry(title).or_default().push(TrackId(path));
                }
            }
            albums.retain(|album| membership.contains_key(&album.title));
            for album in &mut albums {
                album.tracks.clone_from(&membership[&album.title]);
            }
            Ok(albums)
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to list artist albums in genre: {e}"))
        })
    }

    /// One album's tracks with `genre`: track number ascending with missing
    /// numbers first, path tiebreak — the canonical album-track order under
    /// the genre filter. The per-track membership split (`;`-separated
    /// entries) happens in Rust on the genre-bearing rows.
    fn album_tracks_in_genre(
        &self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
    ) -> Result<Vec<Track>, StoreError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks
                 WHERE album_artist_key = ?1 AND album_title_key = ?2
                   AND genre IS NOT NULL AND genre != ''
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC"
            ))?;
            let rows =
                stmt.query_map(rusqlite::params![album_artist, album_title], track_from_row)?;
            let tracks = rows.collect::<Result<Vec<_>, _>>()?;
            Ok(tracks
                .into_iter()
                .filter(|track| {
                    track
                        .metadata
                        .genre
                        .as_deref()
                        .is_some_and(|stored| genre_contains(stored, genre))
                })
                .collect())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to list album tracks in genre: {e}"))
        })
    }

    // --- Entity hit reads (search across Library sections) ------------------

    // Sliced implementations: each read lands here behind its failing
    // store test (search-across-library-entity-columns issue 01).

    fn hit_albums(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Album>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            // One CTE pass: the window of hit albums LEFT JOINed to their hit
            // tracks, so membership arrives in canonical per-album track order
            // and a name-hit album with no matching track still appears. The
            // hit predicate is the union of the album's own name match and a
            // member-track match, both literal `instr` over the
            // write-time-lowercased columns.
            let mut stmt = conn.prepare_cached(
                "WITH window AS (
                    SELECT a.album_artist, a.title, a.year, a.genre
                    FROM albums a
                    WHERE instr(a.album_artist_lower, ?1) > 0
                       OR instr(a.title_lower, ?1) > 0
                       OR EXISTS (
                            SELECT 1 FROM tracks t
                            WHERE t.album_artist_key = a.album_artist
                              AND t.album_title_key = a.title
                              AND instr(t.search_text, ?1) > 0
                          )
                    ORDER BY a.album_artist ASC, COALESCE(a.year, 0) DESC, a.title ASC
                    LIMIT ?2 OFFSET ?3
                 )
                 SELECT w.album_artist, w.title, w.year, w.genre,
                        t.album_title_key, t.path
                 FROM window w
                 LEFT JOIN tracks t
                   ON t.album_artist_key = w.album_artist
                  AND t.album_title_key = w.title
                  AND instr(t.search_text, ?1) > 0
                 ORDER BY w.album_artist ASC, COALESCE(w.year, 0) DESC, w.title ASC,
                          COALESCE(t.track_number, 0) ASC, t.path ASC",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![
                    needle,
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )?;
            // The window is ordered canonically and the LEFT JOIN keeps one
            // row per hit track, so consecutive album rows group naturally.
            let mut albums: Vec<Album> = Vec::new();
            for row in rows {
                let (artist, title, year, genre, hit_path) = row?;
                if albums
                    .last()
                    .is_some_and(|a: &Album| a.artist == artist && a.title == title)
                {
                    if let Some(path) = hit_path {
                        albums
                            .last_mut()
                            .expect("just checked")
                            .tracks
                            .push(TrackId(path));
                    }
                } else {
                    albums.push(Album {
                        artist,
                        title,
                        year: narrow_u32(year),
                        genre,
                        tracks: hit_path.map(|path| vec![TrackId(path)]).unwrap_or_default(),
                    });
                }
            }
            Ok(albums)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list hit albums: {e}")))
    }

    fn hit_albums_count(&self, query: &str) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            conn.query_row(
                "SELECT COUNT(*) FROM albums a
                 WHERE instr(a.album_artist_lower, ?1) > 0
                    OR instr(a.title_lower, ?1) > 0
                    OR EXISTS (
                         SELECT 1 FROM tracks t
                         WHERE t.album_artist_key = a.album_artist
                           AND t.album_title_key = a.title
                           AND instr(t.search_text, ?1) > 0
                       )",
                [needle],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count hit albums: {e}")))
    }

    fn hit_artists(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Artist>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            // Window of hit artists (name match or any album hit) LEFT JOINed
            // to their hit albums, so each artist's hit-album keys arrive in
            // canonical browsing order and a name-hit artist whose albums are
            // not themselves hits still appears with an empty key list.
            let mut stmt = conn.prepare_cached(
                "WITH window AS (
                    SELECT ar.name
                    FROM artists ar
                    WHERE instr(ar.name_lower, ?1) > 0
                       OR EXISTS (
                            SELECT 1 FROM albums a
                            WHERE a.album_artist = ar.name
                              AND (instr(a.album_artist_lower, ?1) > 0
                                   OR instr(a.title_lower, ?1) > 0
                                   OR EXISTS (
                                        SELECT 1 FROM tracks t
                                        WHERE t.album_artist_key = a.album_artist
                                          AND t.album_title_key = a.title
                                          AND instr(t.search_text, ?1) > 0
                                      ))
                          )
                    ORDER BY ar.name ASC
                    LIMIT ?2 OFFSET ?3
                 )
                 SELECT w.name, a.title
                 FROM window w
                 LEFT JOIN albums a
                   ON a.album_artist = w.name
                  AND (instr(a.album_artist_lower, ?1) > 0
                       OR instr(a.title_lower, ?1) > 0
                       OR EXISTS (
                            SELECT 1 FROM tracks t
                            WHERE t.album_artist_key = a.album_artist
                              AND t.album_title_key = a.title
                              AND instr(t.search_text, ?1) > 0
                          ))
                 ORDER BY w.name ASC, COALESCE(a.year, 0) DESC, a.title ASC",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![
                    needle,
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )?;
            // The window is ordered by artist and the LEFT JOIN keeps one row
            // per hit album, so consecutive rows group naturally.
            let mut artists: Vec<Artist> = Vec::new();
            for row in rows {
                let (name, hit_title) = row?;
                if artists.last().is_some_and(|a: &Artist| a.name == name) {
                    if let Some(title) = hit_title {
                        artists
                            .last_mut()
                            .expect("just checked")
                            .albums
                            .push(format!("{name} - {title}"));
                    }
                } else {
                    let key = hit_title.map(|t| format!("{name} - {t}"));
                    artists.push(Artist {
                        name,
                        albums: key.map(|k| vec![k]).unwrap_or_default(),
                    });
                }
            }
            Ok(artists)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list hit artists: {e}")))
    }

    fn hit_artists_count(&self, query: &str) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            conn.query_row(
                "SELECT COUNT(*) FROM artists ar
                 WHERE instr(ar.name_lower, ?1) > 0
                    OR EXISTS (
                         SELECT 1 FROM albums a
                         WHERE a.album_artist = ar.name
                           AND (instr(a.album_artist_lower, ?1) > 0
                                OR instr(a.title_lower, ?1) > 0
                                OR EXISTS (
                                     SELECT 1 FROM tracks t
                                     WHERE t.album_artist_key = a.album_artist
                                       AND t.album_title_key = a.title
                                       AND instr(t.search_text, ?1) > 0
                                   ))
                       )",
                [needle],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count hit artists: {e}")))
    }

    fn album_hit_tracks(
        &self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Result<Vec<Track>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks
                 WHERE album_artist_key = ?1 AND album_title_key = ?2
                   AND instr(search_text, ?3) > 0
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![album_artist, album_title, needle],
                track_from_row,
            )?;
            rows.collect()
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list album hit tracks: {e}")))
    }

    fn album_is_name_hit(
        &self,
        album_artist: &str,
        album_title: &str,
        query: &str,
    ) -> Result<bool, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            let hit: i64 = conn
                .prepare_cached(
                    "SELECT EXISTS(
                        SELECT 1 FROM albums
                        WHERE album_artist = ?1 AND title = ?2
                          AND (instr(album_artist_lower, ?3) > 0
                               OR instr(title_lower, ?3) > 0)
                     )",
                )?
                .query_row(
                    rusqlite::params![album_artist, album_title, needle],
                    |row| row.get(0),
                )?;
            Ok(hit > 0)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to check album name hit: {e}")))
    }

    fn hit_albums_in_genre(
        &self,
        genre: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Album>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            let mut albums = Self::genre_scoped_hit_albums(conn, genre, &needle)?;
            albums = albums.into_iter().skip(offset).take(limit).collect();
            Self::attach_genre_hit_track_ids(conn, genre, &needle, &mut albums)?;
            Ok(albums)
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to list hit albums in genre: {e}"))
        })
    }

    fn hit_albums_in_genre_count(&self, genre: &str, query: &str) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            Ok(Self::genre_scoped_hit_albums(conn, genre, &needle)?.len())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to count hit albums in genre: {e}"))
        })
    }

    fn hit_artists_in_genre(
        &self,
        genre: &str,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Artist>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            // Group the genre-scoped hit albums by artist; the canonical
            // album ordering keeps each artist's keys canonical, so only the
            // name-ascending sort of the artists themselves remains.
            let mut keys_by_artist: HashMap<String, Vec<String>> = HashMap::new();
            for album in Self::genre_scoped_hit_albums(conn, genre, &needle)? {
                keys_by_artist
                    .entry(album.artist.clone())
                    .or_default()
                    .push(format!("{} - {}", album.artist, album.title));
            }
            let mut names: Vec<String> = keys_by_artist.keys().cloned().collect();
            names.sort();
            Ok(names
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|name| Artist {
                    albums: keys_by_artist.remove(&name).unwrap_or_default(),
                    name,
                })
                .collect())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to list hit artists in genre: {e}"))
        })
    }

    fn hit_artists_in_genre_count(&self, genre: &str, query: &str) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            let mut names: Vec<String> = Vec::new();
            for album in Self::genre_scoped_hit_albums(conn, genre, &needle)? {
                if !names.contains(&album.artist) {
                    names.push(album.artist);
                }
            }
            Ok(names.len())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to count hit artists in genre: {e}"))
        })
    }

    fn album_hit_tracks_in_genre(
        &self,
        album_artist: &str,
        album_title: &str,
        genre: &str,
        query: &str,
    ) -> Result<Vec<Track>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks
                 WHERE album_artist_key = ?1 AND album_title_key = ?2
                   AND instr(search_text, ?3) > 0
                   AND genre IS NOT NULL AND genre != ''
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![album_artist, album_title, needle],
                track_from_row,
            )?;
            let tracks = rows.collect::<Result<Vec<_>, _>>()?;
            Ok(tracks
                .into_iter()
                .filter(|track| {
                    track
                        .metadata
                        .genre
                        .as_deref()
                        .is_some_and(|stored| genre_contains(stored, genre))
                })
                .collect())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to list album hit tracks in genre: {e}"))
        })
    }

    fn hit_genre_counts(&self, query: &str) -> Result<Vec<GenreCount>, StoreError> {
        self.with_connection(|conn| {
            let needle = query.to_lowercase();
            // Exactly `genre_counts` semantics over the hit subset: the SQL
            // keeps every hit genre-bearing track, the segments split in Rust.
            let mut stmt = conn.prepare(
                "SELECT genre FROM tracks
                 WHERE instr(search_text, ?1) > 0
                   AND genre IS NOT NULL AND genre != ''",
            )?;
            let rows = stmt.query_map([needle], |row| row.get::<_, String>(0))?;
            let mut counts: HashMap<String, usize> = HashMap::new();
            for row in rows {
                for segment in genre_segments(&row?) {
                    *counts.entry(segment.to_string()).or_insert(0) += 1;
                }
            }
            let mut entries: Vec<GenreCount> = counts
                .into_iter()
                .map(|(genre, tracks)| GenreCount { genre, tracks })
                .collect();
            entries.sort_by(|a, b| a.genre.cmp(&b.genre));
            Ok(entries)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to aggregate hit genres: {e}")))
    }

    // --- Paged browse reads (paginate-browse-columns) ----------------------
    //
    // Bounded windows + authoritative totals for the browse columns and the
    // genre drill-downs, ordered purely in SQL. The `direction` lands in the
    // `ORDER BY` so page offsets stay aligned when the sort reverses.

    /// One bounded window of artists, name-ascending or name-descending per
    /// `direction`, each carrying its album keys in canonical browsing order.
    /// Mirrors [`Self::all_artists`]' grouping over a windowed name list.
    fn artists_window(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Artist>, StoreError> {
        self.with_connection(|conn| {
            let name_order = match direction {
                SortDirection::Ascending => "ASC",
                SortDirection::Descending => "DESC",
            };
            let mut stmt = conn.prepare_cached(&format!(
                "WITH window AS (
                    SELECT name FROM artists
                    ORDER BY name {name_order}
                    LIMIT ?1 OFFSET ?2
                 )
                 SELECT w.name, a.title
                 FROM window w
                 LEFT JOIN albums a ON a.album_artist = w.name
                 ORDER BY w.name {name_order}, COALESCE(a.year, 0) DESC, a.title ASC"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )?;
            // The window is ordered by artist and the LEFT JOIN keeps one row
            // per album, so consecutive rows group naturally into artists
            // carrying their canonical album keys.
            let mut artists: Vec<Artist> = Vec::new();
            for row in rows {
                let (name, title) = row?;
                if artists.last().is_some_and(|a: &Artist| a.name == name) {
                    if let Some(title) = title {
                        artists
                            .last_mut()
                            .expect("just checked")
                            .albums
                            .push(format!("{name} - {title}"));
                    }
                } else {
                    artists.push(Artist {
                        name: name.clone(),
                        albums: title
                            .map(|t| vec![format!("{name} - {t}")])
                            .unwrap_or_default(),
                    });
                }
            }
            Ok(artists)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list artist windows: {e}")))
    }

    fn artists_count(&self) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM artists", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count artists: {e}")))
    }

    /// One bounded window over every album in the flat browsing order (album
    /// artist ascending, year descending with missing years last, then title
    /// ascending) or its exact reversal per `direction`, each carrying its
    /// full track ids in album-track order.
    fn albums_window(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Album>, StoreError> {
        self.with_connection(|conn| {
            let (artist_order, year_order, title_order) = match direction {
                SortDirection::Ascending => ("ASC", "DESC", "ASC"),
                SortDirection::Descending => ("DESC", "ASC", "DESC"),
            };
            let mut stmt = conn.prepare_cached(&format!(
                "WITH window AS (
                    SELECT album_artist, title, year, genre
                    FROM albums
                    ORDER BY album_artist {artist_order}, COALESCE(year, 0) {year_order}, title {title_order}
                    LIMIT ?1 OFFSET ?2
                 )
                 SELECT w.album_artist, w.title, w.year, w.genre,
                        t.album_title_key, t.path
                 FROM window w
                 LEFT JOIN tracks t
                   ON t.album_artist_key = w.album_artist
                  AND t.album_title_key = w.title
                 ORDER BY w.album_artist {artist_order}, COALESCE(w.year, 0) {year_order}, w.title {title_order},
                          COALESCE(t.track_number, 0) ASC, t.path ASC"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )?;
            // The window is ordered canonically and the LEFT JOIN keeps one
            // row per track, so consecutive rows group naturally per album.
            let mut albums: Vec<Album> = Vec::new();
            for row in rows {
                let (artist, title, year, genre, path) = row?;
                if albums
                    .last()
                    .is_some_and(|a: &Album| a.artist == artist && a.title == title)
                {
                    if let Some(path) = path {
                        albums.last_mut().expect("just checked").tracks.push(TrackId(path));
                    }
                } else {
                    albums.push(Album {
                        artist,
                        title,
                        year: narrow_u32(year),
                        genre,
                        tracks: path.map(|p| vec![TrackId(p)]).unwrap_or_default(),
                    });
                }
            }
            Ok(albums)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list album windows: {e}")))
    }

    fn albums_count(&self) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            conn.query_row("SELECT COUNT(*) FROM albums", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| usize::try_from(count).unwrap_or(usize::MAX))
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count albums: {e}")))
    }

    /// One bounded window of genre entries, name-ascending or name-descending
    /// per `direction`, each carrying its per-track count aggregated from the
    /// tracks' own genre metadata exactly like [`Self::genre_counts`].
    fn genres_window(
        &self,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<GenreCount>, StoreError> {
        self.with_connection(|conn| {
            // The entries split in Rust (exactly like `genre_counts`); the
            // window slices the sorted entries afterwards, so the direction
            // reversal is a Rust reorder of the pre-window entries.
            let mut stmt = conn.prepare(
                "SELECT genre FROM tracks
                 WHERE genre IS NOT NULL AND genre != ''",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut counts: HashMap<String, usize> = HashMap::new();
            for row in rows {
                for segment in genre_segments(&row?) {
                    *counts.entry(segment.to_string()).or_insert(0) += 1;
                }
            }
            let mut entries: Vec<GenreCount> = counts
                .into_iter()
                .map(|(genre, tracks)| GenreCount { genre, tracks })
                .collect();
            entries.sort_by(|a, b| a.genre.cmp(&b.genre));
            match direction {
                SortDirection::Ascending => {}
                SortDirection::Descending => entries.reverse(),
            }
            Ok(entries.into_iter().skip(offset).take(limit).collect())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to aggregate genre windows: {e}"))
        })
    }

    fn genres_count(&self) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT genre FROM tracks
                 WHERE genre IS NOT NULL AND genre != ''",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            let mut genres = std::collections::HashSet::new();
            for row in rows {
                for segment in genre_segments(&row?) {
                    genres.insert(segment.to_string());
                }
            }
            Ok(genres.len())
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count genres: {e}")))
    }

    /// One bounded window of artists within `genre`, name-ascending or
    /// name-descending per `direction`, each with its genre-scoped album keys
    /// in canonical browsing order. Mirrors [`Self::artists_in_genre`] over a
    /// windowed name list.
    fn artists_in_genre_window(
        &self,
        genre: &str,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Artist>, StoreError> {
        self.with_connection(|conn| {
            // The genre-scoped (artist, album) membership splits in Rust like
            // `artists_in_genre`; the window slices the name-ascending artists
            // afterwards, so the direction reversal is a Rust reorder.
            let mut stmt = conn.prepare(
                "SELECT album_artist_key, album_title_key, genre
                 FROM tracks
                 WHERE genre IS NOT NULL AND genre != ''",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut keys_by_artist: HashMap<String, Vec<String>> = HashMap::new();
            for row in rows {
                let (artist, title, stored_genre) = row?;
                if genre_contains(&stored_genre, genre) {
                    keys_by_artist
                        .entry(artist.clone())
                        .or_default()
                        .push(format!("{artist} - {title}"));
                }
            }
            let mut names: Vec<String> = keys_by_artist.keys().cloned().collect();
            names.sort();
            match direction {
                SortDirection::Ascending => {}
                SortDirection::Descending => names.reverse(),
            }
            Ok(names
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|name| Artist {
                    albums: keys_by_artist.remove(&name).unwrap_or_default(),
                    name,
                })
                .collect())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to list genre artist windows: {e}"))
        })
    }

    fn artists_in_genre_count(&self, genre: &str) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT album_artist_key, genre FROM tracks
                 WHERE genre IS NOT NULL AND genre != ''",
            )?;
            let pairs = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut artists: Vec<String> = Vec::new();
            for (artist, stored) in pairs {
                if genre_contains(&stored, genre) && !artists.contains(&artist) {
                    artists.push(artist);
                }
            }
            Ok(artists.len())
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count genre artists: {e}")))
    }

    /// One bounded window of one artist's albums holding at least one Track
    /// with `genre`, in canonical browsing order or its exact reversal per
    /// `direction`, each carrying only its matching track ids. Mirrors
    /// [`Self::artist_albums_in_genre`] over the artist's windowed albums.
    fn artist_albums_in_genre_window(
        &self,
        artist: &str,
        genre: &str,
        direction: SortDirection,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Album>, StoreError> {
        self.with_connection(|conn| {
            let (year_order, title_order) = match direction {
                SortDirection::Ascending => ("DESC", "ASC"),
                SortDirection::Descending => ("ASC", "DESC"),
            };
            let mut stmt = conn.prepare_cached(&format!(
                "WITH window AS (
                    SELECT title, year, genre FROM albums
                    WHERE album_artist = ?1
                    ORDER BY COALESCE(year, 0) {year_order}, title {title_order}
                    LIMIT ?2 OFFSET ?3
                 )
                 SELECT w.title, w.year, w.genre, t.album_title_key, t.path, t.genre
                 FROM window w
                 LEFT JOIN tracks t
                   ON t.album_artist_key = ?1
                  AND t.album_title_key = w.title
                 ORDER BY COALESCE(w.year, 0) {year_order}, w.title {title_order},
                          COALESCE(t.track_number, 0) ASC, t.path ASC"
            ))?;
            let rows = stmt.query_map(
                rusqlite::params![
                    artist,
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX),
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )?;
            // The window is ordered canonically and the LEFT JOIN keeps one
            // row per track, so consecutive rows group naturally per album.
            // Only tracks whose own genre carries `genre` join an album's
            // membership — a Metal track never lands in the Rock list.
            let mut albums: Vec<Album> = Vec::new();
            for row in rows {
                let (title, year, album_genre, path, stored_genre) = row?;
                if albums.last().is_some_and(|a: &Album| a.title == title) {
                    if let (Some(path), Some(stored_genre)) = (path, stored_genre)
                        && genre_contains(&stored_genre, genre)
                    {
                        albums
                            .last_mut()
                            .expect("just checked")
                            .tracks
                            .push(TrackId(path));
                    }
                } else {
                    albums.push(Album {
                        artist: artist.to_string(),
                        title,
                        year: narrow_u32(year),
                        genre: album_genre,
                        tracks: path
                            .zip(stored_genre)
                            .filter(|(_, stored)| genre_contains(stored, genre))
                            .map(|(p, _)| vec![TrackId(p)])
                            .unwrap_or_default(),
                    });
                }
            }
            // Drop albums holding no matching track (the genre filter): the
            // LEFT JOIN kept them for ordering, but without a genre-bearing
            // member they have no row in the scoped list.
            albums.retain(|album| !album.tracks.is_empty());
            Ok(albums)
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!(
                "failed to list artist album windows in genre: {e}"
            ))
        })
    }

    fn artist_albums_in_genre_count(&self, artist: &str, genre: &str) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT album_title_key, genre FROM tracks
                 WHERE album_artist_key = ?1 AND genre IS NOT NULL AND genre != ''
                 ORDER BY COALESCE(track_number, 0) ASC, path ASC",
            )?;
            let members = stmt.query_map(rusqlite::params![artist], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut titles: Vec<String> = Vec::new();
            for member in members {
                let (title, stored_genre) = member?;
                if genre_contains(&stored_genre, genre) && !titles.contains(&title) {
                    titles.push(title);
                }
            }
            Ok(titles.len())
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to count artist albums in genre: {e}"))
        })
    }

    /// Escaped prefix existence check over stored track paths.
    fn folder_has_audio(&self, folder: &std::path::Path) -> Result<bool, StoreError> {
        let params = folder_prefix_params(&folder.to_string_lossy());
        self.with_connection(|conn| {
            let exists: i64 = conn
                .prepare_cached(&format!(
                    "SELECT EXISTS(SELECT 1 FROM tracks WHERE {FOLDER_PREFIX_SQL})"
                ))?
                .query_row(rusqlite::params![params[0], params[1], params[2]], |row| {
                    row.get(0)
                })?;
            Ok(exists > 0)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("folder probe failed: {e}")))
    }

    /// Escaped prefix match combined with the flat search's literal
    /// substring semantics over the derived lowercased search text.
    fn folder_has_search_match(
        &self,
        folder: &std::path::Path,
        query: &str,
    ) -> Result<bool, StoreError> {
        let needle = query.to_lowercase();
        let params = folder_prefix_params(&folder.to_string_lossy());
        self.with_connection(|conn| {
            let exists: i64 = conn
                .prepare_cached(&format!(
                    "SELECT EXISTS(
                        SELECT 1 FROM tracks
                        WHERE {FOLDER_PREFIX_SQL} AND instr(search_text, ?4) > 0
                     )"
                ))?
                .query_row(
                    rusqlite::params![params[0], params[1], params[2], needle],
                    |row| row.get(0),
                )?;
            Ok(exists > 0)
        })
        .map_err(|e| StoreError::InvalidOperation(format!("folder search failed: {e}")))
    }

    /// How many tracks live under `folder` (component-wise path prefix,
    /// exactly like [`Self::track_ids_in_folder_tree`]'s membership) — the
    /// per-music-folder count the Settings Library pane shows. One scalar
    /// `COUNT` over the escaped prefix match; a sibling sharing a
    /// byte-prefix (`a` vs `ab`) never matches.
    fn folder_track_count(&self, folder: &std::path::Path) -> Result<usize, StoreError> {
        self.with_connection(|conn| {
            let params = folder_prefix_params(&folder.to_string_lossy());
            conn.query_row(
                &format!("SELECT COUNT(*) FROM tracks WHERE {FOLDER_PREFIX_SQL}"),
                rusqlite::params![params[0], params[1], params[2]],
                |row| {
                    Ok(usize::try_from(row.get::<_, i64>(0)?)
                        .expect("a folder track count fits in usize"))
                },
            )
        })
        .map_err(|e| StoreError::InvalidOperation(format!("failed to count folder tracks: {e}")))
    }

    /// The last completed full library scan's summary from the store's
    /// metadata table. `None` when no scan has ever completed. Values
    /// written by earlier builds carry only the finished nanos — they parse
    /// as zero counts (nothing was recorded about them).
    fn last_full_scan(&self) -> Result<Option<FullScanSummary>, StoreError> {
        self.with_connection(|conn| {
            conn.query_row(
                "SELECT value FROM store_metadata WHERE key = 'last_full_scan'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|stamp| {
                stamp.map(|text| {
                    let mut parts = text.split(':');
                    let nanos: i64 = parts
                        .next()
                        .unwrap_or_default()
                        .parse()
                        .expect("a stored scan stamp starts with an integer");
                    let mut count = |fallback: usize| {
                        parts
                            .next()
                            .and_then(|part| part.parse::<usize>().ok())
                            .unwrap_or(fallback)
                    };
                    FullScanSummary {
                        at: system_time_from_nanos(nanos),
                        files: count(0),
                        errors: count(0),
                    }
                })
            })
        })
        .map_err(|e| {
            StoreError::InvalidOperation(format!("failed to read the last-scan summary: {e}"))
        })
    }

    /// Every path under `folder`, then re-sorted in Rust with the exact
    /// component-wise `Path` comparator the former in-memory tree listing
    /// used — byte-wise SQL order can differ around names that continue a
    /// sibling's name with a low byte (`a.b` vs `a\x`).
    fn track_ids_in_folder_tree(
        &self,
        folder: &std::path::Path,
    ) -> Result<Vec<TrackId>, StoreError> {
        let mut paths = self.folder_paths_under(folder)?;
        paths.sort_by(|a, b| std::path::Path::new(a).cmp(std::path::Path::new(b)));
        Ok(paths.into_iter().map(TrackId).collect())
    }

    /// Tracks whose parent is exactly `folder`. Within one parent directory,
    /// filename byte order equals full-path byte order, so the SQL ordering
    /// reproduces the former number-then-filename sort exactly.
    fn tracks_in_folder(&self, folder: &std::path::Path) -> Result<Vec<Track>, StoreError> {
        let folder_text = folder.to_string_lossy().into_owned();
        let params = folder_prefix_params(&folder_text);
        self.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(&format!(
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to list folder: {e}")))
    }

    /// Direct child directories with audio. The escaped prefix query yields
    /// every stored path two-or-more levels below `folder`; grouping into
    /// first components happens in Rust so the child set, its dedupe, and
    /// its `PathBuf` ordering replicate the former tree walk exactly. A
    /// first component counts as a directory only when something lives
    /// deeper beneath it — the structural stand-in for the former
    /// `is_dir()` stat, which excluded files sitting directly in `folder`.
    fn subdirs_with_audio(&self, folder: &std::path::Path) -> Result<Vec<PathBuf>, StoreError> {
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
    ) -> Result<Vec<Track>, StoreError> {
        let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
        match kind {
            // Exactly the favorited tracks, in the canonical flat ordering
            // (path-ascending, ADR 0003) — a stable order that never
            // depends on insertion or play history.
            SmartPlaylistKind::Favorites => self.with_connection(|conn| {
                let mut stmt = conn.prepare_cached(&format!(
                    "SELECT {TRACK_COLUMNS} FROM tracks
                     WHERE favorite = 1
                     ORDER BY path ASC
                     LIMIT ?1"
                ))?;
                let rows = stmt.query_map([limit_i64], track_from_row)?;
                rows.collect()
            }),
            SmartPlaylistKind::MostPlayed => {
                let mut played: Vec<Track> = self.with_connection(|conn| {
                    let mut stmt = conn.prepare_cached(&format!(
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
                let mut stmt = conn.prepare_cached(&format!(
                    "SELECT {TRACK_COLUMNS} FROM tracks
                     WHERE date_added_nanos IS NOT NULL
                     ORDER BY date_added_nanos DESC, path ASC
                     LIMIT ?1"
                ))?;
                let rows = stmt.query_map([limit_i64], track_from_row)?;
                rows.collect()
            }),
            // Recently Played: newest finished-play first, never-played rows
            // excluded (NULL last_played never qualifies); path tiebreak.
            SmartPlaylistKind::RecentlyPlayed => self.with_connection(|conn| {
                let mut stmt = conn.prepare_cached(&format!(
                    "SELECT {TRACK_COLUMNS} FROM tracks
                     WHERE last_played_nanos IS NOT NULL
                     ORDER BY last_played_nanos DESC, path ASC
                     LIMIT ?1"
                ))?;
                let rows = stmt.query_map([limit_i64], track_from_row)?;
                rows.collect()
            }),
            // Path-ascending unplayed list.
            SmartPlaylistKind::NeverPlayed => self.with_connection(|conn| {
                let mut stmt = conn.prepare_cached(&format!(
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
                    let mut stmt = conn.prepare_cached(&format!(
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
        .map_err(|e| StoreError::InvalidOperation(format!("smart playlist query failed: {e}")))
    }

    /// Every smart playlist's unbounded total, in [`SmartPlaylistKind::ALL`]
    /// order. Each count runs the matching [`Self::smart_playlist`]
    /// membership filter as a scalar `COUNT` (no `LIMIT`); `Lost Gems`
    /// compares against the current clock like the list itself.
    fn smart_list_counts(&self) -> Result<Vec<(SmartPlaylistKind, usize)>, StoreError> {
        self.with_connection(|conn| {
            let scalar = |sql: &str| -> rusqlite::Result<usize> {
                conn.query_row(sql, [], |row| {
                    Ok(usize::try_from(row.get::<_, i64>(0)?)
                        .expect("a smart list count fits in usize"))
                })
            };
            let favorites = scalar("SELECT COUNT(*) FROM tracks WHERE favorite = 1")?;
            let recently_added =
                scalar("SELECT COUNT(*) FROM tracks WHERE date_added_nanos IS NOT NULL")?;
            let most_played = scalar("SELECT COUNT(*) FROM tracks WHERE play_count > 0")?;
            let recently_played =
                scalar("SELECT COUNT(*) FROM tracks WHERE last_played_nanos IS NOT NULL")?;
            let never_played = scalar("SELECT COUNT(*) FROM tracks WHERE play_count = 0")?;

            let now_nanos = nanos_from_system_time(SystemTime::now());
            let threshold_nanos = i64::try_from(LOST_GEMS_THRESHOLD.as_nanos()).unwrap_or(i64::MAX);
            let cutoff = now_nanos.saturating_sub(threshold_nanos);
            let lost_gems = conn.query_row(
                "SELECT COUNT(*) FROM tracks
                 WHERE last_played_nanos IS NULL
                    OR last_played_nanos < ?1
                    OR last_played_nanos > ?2",
                rusqlite::params![cutoff, now_nanos],
                |row| {
                    Ok(usize::try_from(row.get::<_, i64>(0)?)
                        .expect("a smart list count fits in usize"))
                },
            )?;

            Ok(vec![
                (SmartPlaylistKind::Favorites, favorites),
                (SmartPlaylistKind::RecentlyAdded, recently_added),
                (SmartPlaylistKind::MostPlayed, most_played),
                (SmartPlaylistKind::RecentlyPlayed, recently_played),
                (SmartPlaylistKind::NeverPlayed, never_played),
                (SmartPlaylistKind::LostGems, lost_gems),
            ])
        })
        .map_err(|e| StoreError::InvalidOperation(format!("smart list counts failed: {e}")))
    }
}

impl SqliteStore {
    /// Every stored track path under `folder` via [`FOLDER_PREFIX_SQL`] —
    /// including the folder's own exact path when one exists, mirroring
    /// `Path::starts_with`. Callers that must not see it skip it while
    /// grouping (`subdirs_with_audio` drops empty remainders).
    fn folder_paths_under(&self, folder: &std::path::Path) -> Result<Vec<String>, StoreError> {
        let params = folder_prefix_params(&folder.to_string_lossy());
        self.with_connection(|conn| {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT path FROM tracks WHERE {FOLDER_PREFIX_SQL}"
            ))?;
            let rows = stmt
                .query_map(rusqlite::params![params[0], params[1], params[2]], |row| {
                    row.get::<_, String>(0)
                })?;
            rows.collect()
        })
        .map_err(|e| StoreError::InvalidOperation(format!("folder listing failed: {e}")))
    }

    /// Albums that hold `genre` and are hits for `needle`, in canonical
    /// browsing order, with empty membership. A genre-scoped hit is an album
    /// holding a `genre`-bearing track (semicolon-separated entries, matched
    /// in Rust) whose album artist or title matches `needle` (a name hit) or
    /// that owns a genre-bearing member-track hit. Shared by the
    /// genre-scoped hit-album and hit-artist reads; the membership fetch is
    /// separate ([`Self::attach_genre_hit_track_ids`]) so the count path
    /// never pays for it.
    fn genre_scoped_hit_albums(
        conn: &Connection,
        genre: &str,
        needle: &str,
    ) -> rusqlite::Result<Vec<Album>> {
        // One pass over the genre-bearing tracks: every (artist, title) that
        // holds `genre`, plus the subset whose search text also hits.
        let mut stmt = conn.prepare(
            "SELECT album_artist_key, album_title_key, genre, search_text
             FROM tracks
             WHERE genre IS NOT NULL AND genre != ''",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut genre_keys: HashSet<(String, String)> = HashSet::new();
        let mut hit_keys: HashSet<(String, String)> = HashSet::new();
        for row in rows {
            let (artist, title, stored_genre, search_text) = row?;
            if genre_contains(&stored_genre, genre) {
                let key = (artist, title);
                genre_keys.insert(key.clone());
                if search_text.contains(needle) {
                    hit_keys.insert(key);
                }
            }
        }

        // The albums pass, in canonical order, filtered in Rust: a
        // genre-scoped hit is a genre-holding album that hits by name or by
        // a genre-bearing member track.
        let mut stmt = conn.prepare(
            "SELECT album_artist, title, year, genre, album_artist_lower, title_lower
             FROM albums
             ORDER BY album_artist ASC, COALESCE(year, 0) DESC, title ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut albums = Vec::new();
        for row in rows {
            let (artist, title, year, genre, artist_lower, title_lower) = row?;
            let key = (artist.clone(), title.clone());
            if genre_keys.contains(&key)
                && (hit_keys.contains(&key)
                    || artist_lower.contains(needle)
                    || title_lower.contains(needle))
            {
                albums.push(Album {
                    artist,
                    title,
                    year: narrow_u32(year),
                    genre,
                    tracks: Vec::new(),
                });
            }
        }
        Ok(albums)
    }

    /// Fill each album of `albums` with its genre-bearing hit track ids, in
    /// canonical album-track order (the query's hit filter runs in SQL; the
    /// per-track genre membership splits in Rust).
    fn attach_genre_hit_track_ids(
        conn: &Connection,
        genre: &str,
        needle: &str,
        albums: &mut [Album],
    ) -> rusqlite::Result<()> {
        if albums.is_empty() {
            return Ok(());
        }
        let mut stmt = conn.prepare(
            "SELECT album_artist_key, album_title_key, genre, path
             FROM tracks
             WHERE genre IS NOT NULL AND genre != ''
               AND instr(search_text, ?1) > 0
             ORDER BY album_artist_key ASC, COALESCE(track_number, 0) ASC, path ASC",
        )?;
        let rows = stmt.query_map([needle], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let index: HashMap<(String, String), usize> = albums
            .iter()
            .enumerate()
            .map(|(i, album)| ((album.artist.clone(), album.title.clone()), i))
            .collect();
        // The rows arrive grouped by album in canonical album-track order,
        // so appending in row order keeps every album's membership canonical.
        for row in rows {
            let (artist, title, stored_genre, path) = row?;
            if !genre_contains(&stored_genre, genre) {
                continue;
            }
            if let Some(&i) = index.get(&(artist.clone(), title.clone())) {
                albums[i].tracks.push(TrackId(path));
            }
        }
        Ok(())
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
    fn load_settings(&self) -> Result<Settings, StoreError> {
        let scalars = self
            .with_connection(|conn| {
                conn.query_row(
                    "SELECT volume, advanced_mode, high_contrast, replaygain_enabled,
                            shuffle, repeat_mode, browser_layout,
                            skip_hidden_files, scan_formats, read_embedded_artwork,
                            smart_lists_collapsed
                     FROM app_settings WHERE id = 1",
                    [],
                    |row| {
                        let scan_formats: String = row.get(8)?;
                        Ok(ScalarSettings {
                            volume: row.get(0)?,
                            advanced_mode: row.get::<_, i64>(1)? != 0,
                            high_contrast: row.get::<_, i64>(2)? != 0,
                            replaygain_enabled: row.get::<_, i64>(3)? != 0,
                            shuffle: row.get::<_, i64>(4)? != 0,
                            repeat_mode: row.get(5)?,
                            browser_layout: row.get(6)?,
                            skip_hidden_files: row.get::<_, i64>(7)? != 0,
                            scan_formats: scan_formats
                                .split(',')
                                .filter(|extension| !extension.is_empty())
                                .map(str::to_string)
                                .collect(),
                            read_embedded_artwork: row.get::<_, i64>(9)? != 0,
                            smart_lists_collapsed: row.get::<_, i64>(10)? != 0,
                        })
                    },
                )
            })
            .map_err(|e| StoreError::InvalidOperation(format!("failed to load settings: {e}")))?;

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
    fn save_scalars(&mut self, scalars: &ScalarSettings) -> Result<(), StoreError> {
        self.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE;")?;
            let result = conn.execute(
                "UPDATE app_settings
                 SET volume = ?1, advanced_mode = ?2, high_contrast = ?3,
                     replaygain_enabled = ?4, shuffle = ?5, repeat_mode = ?6,
                     browser_layout = ?7, skip_hidden_files = ?8, scan_formats = ?9,
                     read_embedded_artwork = ?10, smart_lists_collapsed = ?11
                 WHERE id = 1",
                rusqlite::params![
                    scalars.volume,
                    i64::from(scalars.advanced_mode),
                    i64::from(scalars.high_contrast),
                    i64::from(scalars.replaygain_enabled),
                    i64::from(scalars.shuffle),
                    scalars.repeat_mode,
                    scalars.browser_layout,
                    i64::from(scalars.skip_hidden_files),
                    scalars.scan_formats.join(","),
                    i64::from(scalars.read_embedded_artwork),
                    i64::from(scalars.smart_lists_collapsed),
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to save settings: {e}")))
    }

    /// Replace the library-path list in one small durable transaction.
    fn save_library_paths(&mut self, paths: &[std::path::PathBuf]) -> Result<(), StoreError> {
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to save library paths: {e}")))
    }

    /// Replace the whole watch-state map in one small durable transaction.
    fn save_watch_states(
        &mut self,
        states: &HashMap<PathBuf, WatchState>,
    ) -> Result<(), StoreError> {
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
        .map_err(|e| StoreError::InvalidOperation(format!("failed to save watch states: {e}")))
    }
}
